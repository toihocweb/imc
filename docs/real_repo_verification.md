# Báo cáo verify imc trên repo React Vite + Java Spring Boot thật

## Repo đã dùng

- **Chính:** https://github.com/RabeMaster/gakkaweo (không starred, được tìm qua topic search)
  - React 19 + **Vite** + TypeScript (frontend/src/**/*.tsx)
  - Java 21 + **Spring Boot** 3.5, Gradle (backend/src/main/java/**/*.java)
  - Python FastAPI ai-service (ai-service/app/main.py)
  - Cấu trúc phân tầng thực tế: controller → service → repository → entity, frontend gọi API qua `apiFetch("/auth/login")`…
- **Phụ (không sweep kỹ):** mrktsm/codecafe (React 18 + Vite, Java Spring Boot, WebSocket OT editor)

Repo clone tại `/root/verify/gakkaweo`, index bằng binary Rust `/root/imc-rs/target/release/imc` (tự build từ source, không dùng bản dựng cũ).

## Cách chạy (để người ta tự lặp lại)

```bash
export IMC_CONFIG=/tmp/vcfg.yaml   # bản config tạm: base_url + api_key từ ~/.imc_config.yaml,
                                  # plus llm.model=claude-haiku-4.5 (endpoint này không có model mặc định,
                                  # mặc định của imc là gemini-3.1-flash-lite → 404; tôi không động đến
                                  # config thật của bạn)
cd /root/verify/gakkaweo
/target/release/imc -C . index .
/target/release/imc -C . stats
/target/release/imc -C . context <file>
/target/release/imc -C . trace "GET /auth/..."
/target/release/imc -C . search "<câu hỏi>"        # -r/--raw lấy hits, --json, không truyền gọi LLM
/target/release/imc -C . memory-add … -p <file>
/target/release/imc setup                          # Index và cấu hình proxy/rule cho Claude Code
```

## Kết quả từng lệnh (trên repo thật)

| Lệnh | Kết quả | Ghi chú |
|------|---------|---------|
| `index` | PASS | 474 file, 1965 symbol, 17501 cạnh, 2125 chunk; không panic trên mã tiếng Hàn |
| `stats` / `projects` | PASS | `-C` bắt buộc (theo quy ước cũ) |
| `read` | PASS | `--limit`/`--offset`/`--symbols` chạy đúng, `--json` vẫn in text (cờ toàn cục vô hiệu đối với `read`, giống `context`) |
| `find` | PARTIAL | `imc find <đường dẫn file>` đúng; `imc find AuthService` (tên symbol) báo "File not found" — giống Python (`find` là reader, không phải tra symbol; dùng `graph`/`search`) |
| `context` | PASS | Java controller: endpoints có prefix `/auth`, dependencies, operations. **Byte-identical với Python** trên 4 file (AuthController.java, api.ts, ai-service main.py, HomePage.tsx) |
| `trace` | PASS | Route target → handler → callees; `--callers` blast radius; `--both`; `trace "POST /auth/login"` chạy được; route-node trùng handler hiển thị `(cycle)` (thuyết minh: resolución tên trống, xem dưới) |
| `callers` / `callees` | PASS | alias của `trace` với hướng đã chọn |
| `search` (raw) | PASS | Không còn panic trên nội dung tiếng Hàn; câu rác trả rỗng cả 2 legs |
| `search` (agentic/LLM) | PASS (với config model) | Cần `llm.model=claude-haiku-4.5` trong config tạm; trả lời có trích dẫn, `−r/--json` không gọi LLM, câu rác không gọi LLM, LLM kết nối lỗi degrade về raw hits (rc=0), LLM trả lỗi rc=1 |
| `graph` | PASS | `graph getCurrentUser` in definición + callees/callers; symbol không tồn tại bây giờ ra **rc=1** (đã sửa) |
| `memory-add/-list/-search/-delete` | PASS | Lưu/truy vấn memory theo file, có điểm số |
| `setup` | PASS | Viết `.claude/settings.local.json` + `imc-rule.md` vào repo |

## Bug tìm được và đã sửa (trong session này)

1. **Panic toàn bộ `imc index` trên repo có chữ không-ASCII** (`crates/imc-parser`: cắt chunk bằng *byte offset* 4096 → `start byte index is not a char boundary`; repo tiếng Hàn làm hỏng cả index: 474 file → 0 symbol, 0 chunk). Sửa: cắt theo ranh giới ký tự (char boundary), thêm bài kiểm thử `chunk_utf8_boundary.rs`. Giống Python không bao giờ lỗi này (Python cắt bằng ký tự). — **A: lỗi thực của port**.
2. **Panic `imc search -r`** (`crates/imc-search`: snippet cắt byte index không check ranh giới). Sửa + kiểm thử.
3. **Panic khi trích dẫn các từ khóa trong chunk** (hàm `mentions` lặp `text[start..]`): sửa bằng `match_indices`.
4. **`imc index` index các file Python loại bỏ** (không đọc `.gitignore` → `indexmycode.egg-info/entry_points.txt` thậm chí xếp trước `main/cli/main.py` cho câu "cli entry point argument parsing"). Sửa: dùng crate `ignore` đọc `.gitignore` (có negation `!ui/dist/`), thêm 2 kiểm thử manifest. Xác minh: egg-info/build/.idea/.git = 0 file trong index mới; danh sách file Rust giờ gần bằng Python (183 vs 190, chênh lệch là các file không theo git/dự án khác snapshot). — **A: pérdida hành vi Python**.
5. `graph <symbol không tồn tại>` in lỗi nhưng ra **rc=0** → sửa thành **rc=1** (giống `trace`). — **B: nhất quán CLI**.
6. `imc config` mask giá trị non-ASCII/ngắn panic (byte slice trên api_key) + `strip_trailing_date` của model id có chữ Hán panic → sửa cả hai + kiểm thử. — **B: lỗi chạy thực**.

## Giới hạn đã biết (đã ghi vào docs/parity_audit.md, không sửa vì phức tạp hoặc bằng thiết kế)

- Callees định bằng **tên trống** → liên kết sai ngôn ngữ (ví dụ Java test gọi `adminUserService.banUser` được gán sang hàm TypeScript `banUser` trong `frontend/.../api.ts`) — Python cũng không xử lý được Java caller này (đánh "unresolved"), Rust ít nhất lấy được cái tên; đây là giới hạn được ghi sẵn.
- `graph <fully-qualified::name>` không nhận (chỉ nhận tên ngắn); `trace` đã xử lý route target `GET /auth/login`.
- Cờ `--json` toàn cục với `context`/`read` vẫn in text (Python báo lỗi cờ không nhận diện; port importer im lặng). Ghi là known inert flag.
- Ngưỡng vector (cosine floor 0.30) được đo trên 2 corpus: câu rác peak 0.26–0.396, câu thực 0.33–0.67 — **không có ngưỡng nào tách hoàn toàn được** hai cấp trên cả hai corpus, nên khi BM25 không có hit nào tôi nâng ngưỡng vector lên 0.45 (câu rác peak 0.396 bị lọc, câu thực 0.45+ vẫn qua). Đo lường chi tiết lưu tại `/root/verify/cos_dist.py`, `/root/verify/junk_separation.py`.
- Bỏ qua bước định vị theo chỉ số ngữ nghĩa (Python `locate_target_file` step 3) — đã ghi là deliberately omitted.

## Con số đo được (chạy thật, không đoán)

- Unit tests: **315 passed / 0 failed** (toàn workspace, bao gồm 7 bài kiểm thử mới trong session này)
- E2E suite: **64/64 passed** (trước 49; thêm suite K cross-boundary route và L agentic search với stub LLM cục bộ, không cần mạng/key)
- Benchmark 25 câu trên `/root/imc`: BM25 14/18/19 (MRR 0.838); hybrid 13/19/21 (MRR 0.767); câu rác 3 câu **trả rỗng ở cả 2 legs** trên cả 2 repo
- qna_report: 32 trích dẫn, 0 sai, 0 chưa kịp /10 câu
- prompt_eval (3 lần): grounded **8.0/8**, key files 9/13, invalid citations **0**, từ chối 1/1
- So sánh byte: `context` của Rust **byte-identical Python** trên 4 file của repo thật

## Cảnh báo về cấu hình của bạn

`~/.imc_config.yaml` và `~/.indexer_config.yaml` đều có `llm.base_url` + `llm.api_key` nhưng **thiếu `llm.model`** → endpoint của bạn không nhận model mặc định `gemini-3.1-flash-lite` (404). Để `imc search` (dùng LLM) hoạt động, chạy:

```bash
imc config set llm.model claude-haiku-4.5
```

Tôi **không** tự động sửa file config thật của bạn; tất cả kiểm tra ở trên dùng `/tmp/vcfg.yaml` (sao chép base_url/key, thêm model), không in ra key anywhere.

## Tệp đính kèm

Báo cáo này gửi kèm dạng tệp (`MEDIA:` đường dẫn dưới đây). Mã đã commit vào `/root/imc-rs` (trừ `crates/imc-proxy-types/anthropic.rs` + `tests/` — để lại theo thỏa thuận session trước, đã test xong, chưa commit).

---

Người dùng yêu cầu "tự test tự fix, không cần hỏi" — mọi sửa ở trên đã chạy test/e2e/bench thật trước khi báo cáo.
