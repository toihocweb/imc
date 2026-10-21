# Agent memory systems — research notes for imc self-learning memory

Ngày: 2026-10-21. Mục tiêu: xác định "khi nào / cách gì" một thông tin đáng lưu, để thiết kế `imc-memory` (Rust) + `imc-proxy` (auto-learning).

Trạng thái code hiện tại (đã đọc):
- `crates/imc-memory/src/extract.rs` — 1 LLM call, prompt ngắn, JSON `{"memories":[...]}`, gate `is_correction`/`has_failure`, `redact_sensitive`.
- `crates/imc-memory/src/lib.rs::add` — dedup chính xác theo `content_norm` (skip), near-dup (token-overlap) → **reinforce** memory cũ (`use_count+1`, `last_used`, `importance = MAX`).
- `crates/imc-proxy/src/imc_memory/learning.rs` — gate → cooldown 180s/project → `spawn_blocking` → `learn_from_exchange`. `is_worth_learning = is_correction(query) || has_failure(answer) || has_failure(query)`.
- Python gốc: `main/memory/extractor.py`, `main/memory/auto_learning.py` (`bg_extract` chưa từng được gọi).

---

## 1. Mem0 — quy tắc add/update/skip/no-op

**Phiên bản cũ (V2 / paper 2504.19413)** — 2 LLM call mỗi lần ghi:
1. Call #1: extract candidate facts (`FACT_RETRIEVAL_PROMPT`) — prompt dạng few-shot, ví dụ nhỏ nhất "Hi." → `{"facts": []}` (dạy model biết từ chối).
2. Call #2: `get_update_memory_messages()` với `DEFAULT_UPDATE_MEMORY_PROMPT` — model trả JSON `{"memory":[{"id","text","event":"ADD|UPDATE|DELETE|NONE","old_memory"}]}`. Quy tắc gốc: ADD = có thông tin mới; UPDATE = đã có nhưng khác/chi tiết hơn; DELETE = mâu thuẫn; NONE = đã có hoặc không liên quan.
   - Source: `https://raw.githubusercontent.com/mem0ai/mem0/main/mem0/configs/prompts.py` (hàm `get_update_memory_messages`, `DEFAULT_UPDATE_MEMORY_PROMPT`).
   - Vấn đề đo được: quyết định LLM không ổn định giữa các run (một run UPDATE, run sau DELETE+ADD) — phân tích tại `https://www.southbridge.ai/blog/mem0-technical-analysis-report`.
   - Ghi chú trong repo: `DEFAULT_UPDATE_MEMORY_PROMPT` / `get_update_memory_messages` hiện **dead code** — không có call site — discussion `https://github.com/mem0ai/mem0/discussions/6726`.

**Phiên bản mới (V3)** — bỏ hẳn UPDATE/DELETE:
- "Single-pass ADD-only extraction (one LLM call, no UPDATE/DELETE)" → `add()` chỉ trả event `ADD`.
- Pipeline: retrieve top-10 memory liên quan (context dedup) → 1 LLM call extract → batch embed → **hash-based dedup (MD5, chặn exact duplicate)** → insert.
- Lý do (chính thức): "The model spends its capacity on understanding the input rather than diffing against existing state."
- Số đo (self-reported): LoCoMo 71.4 → **91.6** (+20 điểm); LongMemEval 67.8 → **93.4** (+26 điểm); extraction latency ~½.
  - Nguồn: `https://docs.mem0.ai/migration/oss-v2-to-v3`, `https://docs.mem0.ai/changelog/sdk`, `https://docs.mem0.ai/core-concepts/how-it-works`.
- Paper gốc (LOCOMO, so với OpenAI Memory): **+26% LLM-as-Judge**, p95 latency **−91%**, token **−90%**: `https://arxiv.org/abs/2504.19413` (lưu ý: số tự báo, chưa có replication độc lập; LOCOMO chỉ ~1.7k QA).
- V3 prompt (`ADDITIVE_EXTRACTION_PROMPT`) có các ràng buộc đáng copy cho imc:
  - Input gồm: New Messages / Summary / **Recently Extracted Memories** (≤20, "primary deduplication reference") / **Existing Memories** (chỉ dùng để dedup+link, "do NOT extract new memories from Existing Memories").
  - "when in doubt, extract" — memory thừa rẻ hơn memory thiếu.
  - Anti first-topic dominance: turn nhiều topic → checklist; turn ≥10 message thường ra 5–15 memories, <3 thì đọc lại.
  - Ngân sách **15–80 từ / memory** (tối đa 100), không được bỏ proper noun / số / ngày.
  - Temporal grounding: mốc ngày tương đối phải resolve theo **ngày của cuộc hội thoại**, không theo ngày xử lý.
  - Attribution: phân biệt "User stated X" vs "User was recommended X".
  - Nguồn: `https://raw.githubusercontent.com/mem0ai/mem0/main/mem0/configs/prompts.py` + `https://www.memoryplugin.com/wiki/smart-extraction.html`.

**Bài học cho imc:** Mem0 đã *đo* rằng để LLM diff ADD/UPDATE/DELETE là đắt và không ổn định. imc nên giữ hướng **add-only + read-time ranking**, không cần LLM quyết định UPDATE.

## 2. Anthropic — memory tool + context engineering

**Memory tool** (`{"type":"memory_20250818","name":"memory"}`), client-side: `view / create / str_replace / insert / delete / rename` trên `/memories`.
- Nguồn: `https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool`
- Hành vi quan trọng:
  - System prompt tự động chèn: "ALWAYS VIEW YOUR MEMORY DIRECTORY BEFORE DOING ANYTHING ELSE… As you make progress, record status/progress/thoughts in your memory. ASSUME INTERRUPTION."
  - Claude tự quyết định *write ở đâu / khi nào*: ghi vào file trong `/memories`, dùng `str_replace`/`delete`/`rename` để dọn file cũ — **không có heuristic bắt buộc**, mô hình tự quyết.
  - Bảo trì: "keep its content up-to-date, coherent and organized… You can rename or delete files that are no longer relevant. **Do not create new files unless necessary.**"
  - Chặn ghi: "Only write down information relevant to <topic>".
  - Bảo mật/giới hạn do **app chủ quản**: cap kích thước file, cap output của `view` (model phân trang bằng `view_range`), **periodically delete memory files not accessed for a long time**, path-traversal protection, redact secrets.
  - `view` file >16.000 ký tự bị truncate → phải phân trang.
  - Pattern multi-session: session đầu khởi tạo memory files (progress log + feature checklist), session sau đọc lại, session cuối cập nhật.

**Context engineering guide** (`https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents`):
- "Structured note-taking, or agentic memory" là 1 trong 3 kỹ thuật cho long-horizon (cùng compaction và multi-agent).
- Nguyên tắc: context là tài nguyên hữu hạn → "find the smallest set of high-signal tokens". Ghi memory ra ngoài context để sống sót compaction.

**Effective harnesses for long-running agents** (`https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents`): initializer agent tạo `init.sh` + `claude-progress.txt` + git commit; coding agent đọc progress/git log lúc bắt đầu session và ghi progress + commit lúc kết thúc.

**Claude Code auto memory** (`https://code.claude.com/docs/en/memory`) — gần nhất với imc:
- "notes Claude writes itself **based on your corrections and preferences**" ⇒ trigger ngữ nghĩa là **correction/feedback**, không phải mỗi turn.
- 4 loại note (frontmatter `type`): `user` (role/prefs), **`feedback` (corrections bạn đưa + approach bạn xác nhận)**, `project` (việc/deadline/decision không suy ra được từ code/git), `reference` (issue tracker/dashboard).
- **Skip rules (rất sát với imc):** "Claude skips anything it can derive from the codebase, such as architecture, file paths, or debugging fixes. It also skips anything your CLAUDE.md files already say. **Claude doesn't save something every session.** It decides what's worth remembering based on whether the information would be useful in a future conversation."
- Lưu trữ: `~/.claude/projects/<project>/memory/` — `MEMORY.md` là **index 1 dòng/memory**, chi tiết nằm ở file topic riêng.
- **Giới hạn kích thước:** chỉ **200 dòng hoặc 25KB** đầu `MEMORY.md` được load mỗi session; file vượt → Claude Code trả lỗi bắt Claude viết lại index. Topic file không auto-load, đọc on-demand.
- Khi file gần limit → Claude Code nhắc Claude rút gọn (1 dòng/entry, ghi chi tiết ra topic file, merge/drop entry cũ). Entry có `modified` timestamp ISO 8601 để biết độ mới.

## 3. Letta (MemGPT) — core / recall / archival

- Nguồn: `https://docs.letta.com/configuration/memory/index.md`, `https://www.letta.com/blog/sleep-time-compute/`, `https://www.letta.com/blog/introducing-the-agent-development-environment/`
- **Core memory**: các block luôn nằm trong prompt, có `id/label/value/limit/description`; agent tự sửa bằng `core_memory_append` / `core_memory_replace`; mặc định block `human` + `persona`. Block có **character limit**.
- **Recall memory**: log toàn bộ lịch sử hội thoại (DB), không agent sửa được; search bằng `conversation_search`.
- **Archival memory**: vector store tự do, ghi bằng `archival_memory_insert`, đọc bằng `archival_memory_search`; agent thấy *memory statistics* trong context để biết khi nào đào.
- **Sleep-time / Dreaming**: agent thứ hai chạy nền, **không** có quyền sửa core memory của primary agent (primary bị cắt tool), chỉ sleep-time agent được gắn `memory_insert/replace/rethink`. Trigger: sau N bước agent, hoặc khi context bị compact, hoặc theo lịch. Kết quả: memory consolidation/dedup/reorg diễn ra ngoài latency path. (`https://forum.letta.com/t/sleeptime-agents-for-memory-consolidation-best-practices-guide/154` — search trước khi insert; nhịp dedup mỗi run, consolidation cuối session, reorganize tuần.)
- Bài học: **tách đường ghi nóng khỏi đường hợp nhất (consolidate)**, và luôn "search trước khi insert".

## 4. Zep / Graphiti — dedupe + update + invalidate

- Paper: `https://arxiv.org/abs/2501.13956`; tài liệu: `https://getzep-graphiti.mintlify.app/concepts/temporal-model`; code dedupe: `https://github.com/getzep/graphiti/blob/main/graphiti_core/prompts/dedupe_edges.py`
- **Bi-temporal**: mỗi fact-edge mang 4 mốc: `valid_at`/`invalid_at` (event time) + `created_at`/`expired_at` (transaction time). Query "đúng lúc nào" trả lời được cả "thế giới" lẫn "hệ thống tin gì".
- **Invalidate, không delete**: khi có mâu thuẫn → edge cũ `invalid_at = new.valid_at`, `expired_at = now`; edge mới tạo ra. Giữ lịch sử.
- **Chia việc LLM/code**: LLM quyết định *duplicate vs contradiction* (judgement ngôn ngữ); code lo *timestamp/interval* (thuần toán). "pairs whose validity windows do not overlap are skipped".
- Prompt dedupe (rất đáng học): `duplicate_facts` chỉ trong EXISTING FACTS; `contradicted_facts` trong cả hai list. Ràng buộc:
  - "NEVER mark facts as duplicates if they have key differences, particularly around numeric values, dates, or key qualifiers."
  - "NEVER mark facts as contradicted unless the new fact explicitly makes the old fact false."
  - "Do NOT infer contradiction from recency alone."
  - Ví dụ: "Alice works at Acme as a software engineer" vs "…as a senior engineer" = **contradiction, không phải duplicate**.
- Entity resolve: exact normalized name → entropy gate (từ chối fuzzy-match tên ngắn/low-info) → MinHash/LSH với **Jaccard ≥ 0.9** → chỉ case mơ hồ mới gọi LLM. (`https://www.memoryplugin.com/wiki/updates-and-conflicts.html`)
- Kết quả đo: DMR **94.8%** (MemGPT 93.4%); LongMemEval-S GPT-4o **71.2% vs 60.2% full-context**, context 1.6k vs 115k token, latency 2.58s vs 28.9s. **Điểm yếu đo được:** single-session assistant questions **thấp hơn baseline ~17.7%** — cấu trúc temporal làm nặng khi không có temporal reasoning. Cost ghi: nhiều LLM call/episode (extract entity, resolve, extract edge, detect contradiction, extract timestamp) → phải off critical path.

## 5. Hệ thống trigger extraction chỉ khi có correction/failure — và số đo

Đây là câu hỏi trung tâm: **có, và đây là hướng có bằng chứng đo được**.

| Hệ thống | Trigger | Cơ chế | Số đo |
|---|---|---|---|
| **Claude Code auto memory** | Correction/preference của user (semantic, model tự quyết) | Model tự ghi file, có `type: feedback` | Không công bố số; tài liệu nói rõ "doesn't save something every session" |
| **imc (hiện tại)** | Regex `is_correction` / `has_failure` | 1 LLM call + near-dup reinforce | Chưa có eval |
| **Reflexion** (Shinn 2023) | **Sau mỗi trial thất bại** | Self-reflection → episodic memory buffer → prepend vào prompt lần sau | HumanEval **91% pass@1 vs 80%** GPT-4; AlfWorld **+22 điểm tuyệt đối** sau 12 trial; HotPotQA **+20%**. Ablation: bỏ long-term memory giảm lợi ích đáng kể; structured reflection hơn raw trajectory replay 8–12% (`https://bytez.com/docs/neurips/70114/paper`) |
| **ToolReflection** (REALM 2025) | Lỗi tool call | Bổ sung dữ liệu lỗi tự sinh + correction | ToolAlpaca: +14% (simulated) / +10.5% (real); GPT4Tools-OOD **+25.4%**, OOD-Hard **+56.2%**, Multistep-100 +4% (`https://aclanthology.org/2025.realm-1.14/`) |
| **ReMe** (AgentScope, ACL 2026 Findings) | **Failure-aware reflection**: phân tích trial thất bại → retry; **chỉ giữ lesson nếu retry thành công**, nếu không thì **discard** (không làm bẩn pool) | Utility-based refinement + pruning theo utility | BFCL-V3, Qwen3-8B: **failure 62 → 47** (sửa 17 lỗi baseline, chỉ tạo 2 lỗi mới) (`https://arxiv.org/html/2512.10696v1`, `https://reme.agentscope.io/en/benchmarks/toolmemory`) |
| **TALE** | Học từ feedback lỗi của chính mình | progress-aware reward | BFCL-v3 **+6.4%** task success, giảm số lần thử (`https://openreview.net/pdf?id=4V0cZkWT1A`) |
| **PALADIN** | Self-correcting sau cascade failure | Trajectory-level recovery | Recovery Rate **+13.6%** trung bình, tương đối 57–66% vs vanilla (`https://www.alphaxiv.org/abs/2509.25238`) |
| **VRL-Bench** (phản chứng) | Reflexion-style update sau trial fail | — | **Không phải verbal-memory nào đều thắng retry không-memory**: Reflexion tăng ở model này, **giảm** ở model khác (MiniWoB +1.6pp [−0.6,+4.1]; WebShop GPT-5.4 nano +16pp [+8,+25]) (`https://arxiv.org/html/2609.12404v1`) |
| **Mem0 v3 (đối chứng)** | Extract mỗi turn, add-only | 1 LLM call + MD5 dedup | LoCoMo +20 điểm, LongMemEval +26 điểm |

Kết luận phần 5: **"chỉ học khi có correction/failure" là hướng đúng** (Reflexion/ReMe/ToolReflection đều dương), **nhưng phải có điều kiện thành công** — ReMe chỉ giữ lesson khi retry thành công; VRL-Bench cảnh báo memory sai còn tệ hơn không có memory. Đây là lý do imc nên thêm "verified" semantics.

## 6. LangMem — hot path vs background

- `https://www.langchain.com/blog/langmem-sdk-launch`, `https://github.com/langchain-ai/langmem/blob/main/docs/docs/concepts/conceptual_guide.md`, `https://langchain-ai.github.io/langmem/reference/memory/`, `https://langchain-ai.github.io/langmem/guides/delayed_processing/`
- 3 loại: **semantic** (facts, dạng Collection hoặc Profile), **episodic** (trải nghiệm), **procedural** (prompt rules, qua `create_prompt_optimizer`).
- `create_memory_manager(..., enable_inserts=True, enable_updates=True, enable_deletes=False)` — manager gọi LLM emit **tool call insert/update/delete trong 1 turn**, diff với store rồi mới ghi.
- **Profile pattern**: `enable_inserts=False` → chỉ update 1 bản ghi duy nhất (theo dõi trạng thái hiện tại) thay vì tích tụ bản ghi mới. (`https://langchain-ai.github.io/langmem/guides/manage_user_profile/`)
- **Hot path vs background**: hot = agent tự gọi tool lúc nói (chậm, có latency); background = `create_memory_store_manager` + `ReflectionExecutor` chạy sau, **debounce**: message mới đến thì cancel task cũ, reschedule (delay khuyến nghị 30–60 phút trong production) → 1 extraction cho cả đoạn hội thoại thay vì nhiều lần trên context dở dang.
- Procedural optimizer: `metaprompt` (reflection + thinking rồi meta-prompt đề xuất update), `gradient` (critique → proposal), `prompt_memory` (1 bước).

---

## Khuyến nghị cho imc (10 điểm, cụ thể)

1. **Giữ mô hình add-only + read-time ranking; KHÔNG đưa UPDATE/DELETE vào LLM call.** Mem0 đã đo được việc bỏ 2-call diff là +20/+26 điểm (https://docs.mem0.ai/migration/oss-v2-to-v3). imc hiện đã đúng hướng: `add()` skip exact-dup, near-dup → reinforce. Giữ nguyên, đừng thêm LLM "decide to update".
2. **Bổ sung 3 tầng dedup theo mẫu Mem0 V3**: (a) exact `content_norm` (đã có), (b) hash MD5 của normalized text (giống Mem0) để chặn dup chéo scene, (c) **truyền "recently extracted" (≤20) + "existing" vào prompt** để LLM tự trả `[]` thay vì tạo bản sao. Hiện `existing_as_text` lấy 200 memory toàn store — nên lấy **top-K theo similarity của query/scene**, không phải 200 dòng bất kỳ.
3. **Sửa prompt extractor theo checklist Mem0 V3**: (i) "when in doubt, extract"; (ii) **ngân sách 15–80 từ/memory**, không bỏ proper noun/số/ngày; (iii) **temporal grounding** — mốc ngày tương đối resolve theo ngày hội thoại, ghi kèm timestamp nguồn; (iv) phân biệt "User stated X" vs "User was recommended X"; (v) anti first-topic dominance: ≥10 message → kỳ vọng 5–15 memories, <3 thì đọc lại. Đây là các thay đổi prompt rẻ, hiệu quả cao.
4. **Bổ sung "skip" rules giống Claude Code auto memory vào prompt**: bỏ những gì suy ra được từ codebase (kiến trúc, đường dẫn file, fix debug) và bỏ những gì đã nằm trong CLAUDE.md/README. imc đã có phần này trong `EXTRACTION_PROMPT` — hãy làm rõ thêm "không lưu thông tin đã có trong file hướng dẫn dự án".
5. **Thêm trạng thái "verified" cho memory học từ failure — theo ReMe.** Memory được sinh từ một lần failure chỉ nên được **promote** (tăng importance) sau khi lần chạy lại với lệnh thay thế **thành công**; nếu retry thất bại thì **loại bỏ** (không để đầy store bằng lời khuyên sai). ReMe đo được BFCL-V3 failure 62→47 với chính sách này (https://arxiv.org/html/2512.10696v1). Hiện `auto_learning.py` Python đã có khung `explicit_success` / `_commands_related` / fingerprint — hãy port sang Rust thay vì viết lại.
6. **Tách 2 loại memory theo nguồn, vì chúng xử lý khác nhau:** (a) **feedback/correction** của user → ghi ngay (hot path), vì đây là tín hiệu mạnh nhất, tương đương `type: feedback` của Claude Code; (b) **failure-pattern/tool-bug** → ghi vào kho riêng, chỉ promote sau khi verified (như #5). Hiện imc gộp cả hai vào `work_fact`/`work_method` — nên thêm `mem_type` riêng (vd `tool_gotcha`) để retrieval không trộn.
7. **Sửa gate để bắt được signal thật, không chỉ regex.** Hiện `is_worth_learning` chỉ soi **user message + assistant text**, bỏ qua **tool_result** — mà tool_result mới là nơi chứa "command not found". Claude Code auto memory cũng chỉ dựa vào feedback của user. Khuyến nghị: (i) quét `tool_result` có `is_error=true` hoặc non-zero exit code (Python `auto_learning.extract_tool_outcomes` đã làm việc này — `is_error` + `_FAILURE_PATTERNS` + `exit code N` + `explicit_success`); (ii) giữ regex như **pre-filter rẻ** (đúng như hiện tại), nhưng **mở gate khi**: user xác nhận ("yes, that works", "ok, đúng rồi") → đây là tín hiệu *verified* mạnh nhất; (iii) coi "sửa lại lần thứ hai" (cùng lệnh fail 2 lần) là trigger.
8. **Debounce theo LangMem, không theo cooldown cố định.** `COOLDOWN = 180s` hiện tại **bỏ mất** correction thứ 2 trong vòng 3 phút (đúng cái user sẽ phàn nàn). Thay bằng `ReflectionExecutor`-kiểu: giữ pending task theo project, message mới → cancel + reschedule, và **gộp nhiều message** vào 1 extraction (context dở dang → memory sai). Nguồn: https://langchain-ai.github.io/langmem/guides/delayed_processing/
9. **Giới hạn kích thước + cơ chế expiry kiểu Anthropic/Claude Code.** Claude Code: index `MEMORY.md` 200 dòng/25KB, topic file tách riêng, entry có `modified` timestamp. Anthropic memory tool: app phải **cap kích thước file**, **xóa file không được truy cập lâu**, cap output `view`. imc nên: (i) thêm cột `updated_at`/`last_used` (đã có `last_used`) và quy tắc **TTL/eviction** cho memory không được recall trong N ngày; (ii) giới hạn số memory inject mỗi lần search (top-K theo importance×recency×relevance, kiểu Generative Agents poignancy + exponential decay).
10. **Cần một eval harness nhỏ trước khi tune tiếp.** Theo VRL-Bench (https://arxiv.org/html/2609.12404v1): memory-based learning **không** đều thắng retry không-memory, hướng gain đổi chiều theo model. Vì vậy: dựng bộ ~50–100 tình huống (correction, tool failure, câu hỏi thường) và đo **precision của memory** (mỗi memory có đúng, có được recall lại đúng lúc không) + **recall sau khi gặp lại lỗi** (lần 2 có dùng được memory không). Chỉ số đo được từ chính các hệ thống trên: ReMe (failure count), ToolReflection (success rate), Reflexion (pass@1), Mem0 V3 (LoCoMo/LongMemEval). Không có eval thì mọi prompt-tuning là đoán.

### Thứ tự triển khai đề xuất
- P0 (rẻ, tác động cao): #1 giữ, #3 prompt checklist + temporal grounding, #5 verified/promote từ ReMe, #7 gate dựa trên tool_result.
- P1: #2 dedup 3 tầng + top-K existing, #8 debounce, #9 TTL/size cap.
- P2: #4 skip rules chi tiết, #6 tách mem_type, #10 eval harness.

### Nguồn chính
- Mem0: https://docs.mem0.ai/core-concepts/how-it-works · https://docs.mem0.ai/migration/oss-v2-to-v3 · https://arxiv.org/abs/2504.19413 · https://raw.githubusercontent.com/mem0ai/mem0/main/mem0/configs/prompts.py
- Anthropic: https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool · https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents · https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents · https://code.claude.com/docs/en/memory
- Letta: https://docs.letta.com/configuration/memory/index.md · https://www.letta.com/blog/sleep-time-compute/
- Zep/Graphiti: https://arxiv.org/abs/2501.13956 · https://getzep-graphiti.mintlify.app/concepts/temporal-model · https://github.com/getzep/graphiti/blob/main/graphiti_core/prompts/dedupe_edges.py
- LangMem: https://www.langchain.com/blog/langmem-sdk-launch · https://langchain-ai.github.io/langmem/reference/memory/ · https://langchain-ai.github.io/langmem/guides/delayed_processing/
- Failure-driven learning: https://bytez.com/docs/neurips/70114/paper (Reflexion) · https://aclanthology.org/2025.realm-1.14/ (ToolReflection) · https://arxiv.org/html/2512.10696v1 (ReMe) · https://openreview.net/pdf?id=4V0cZkWT1A (TALE) · https://www.alphaxiv.org/abs/2509.25238 (PALADIN) · https://arxiv.org/html/2609.12404v1 (VRL-Bench, phản chứng)
- Tổng quan đối chiếu: https://www.memoryplugin.com/wiki/smart-extraction.html · https://www.memoryplugin.com/wiki/updates-and-conflicts.html · https://www.memoryplugin.com/wiki/temporal-memory.html
