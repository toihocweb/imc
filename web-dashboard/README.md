# imc Dashboard

Web dashboard for **imc** — browse indexed projects, run hybrid search, trace call graphs,
inspect memories, and exercise the CCR compression store, all from a browser.

Built with Next.js 16 (App Router) + shadcn/ui components (hand-written, no CLI) + Tailwind v4.

## Pages

| Route | What it does |
|---|---|
| `/` | Overview: all registered projects with file/symbol/edge/chunk counts, plus totals |
| `/projects` | Full project table (path, counts, DB size) |
| `/projects/[id]` | Project detail with tabs: search, trace (callers/callees tree), memories |
| `/search` | Hybrid search with a project picker; shows file:lines, symbol, kind, score, source |
| `/trace` | Call-hierarchy tracer: callees / callers / both, rendered as an indented tree |
| `/memory` | List and add project memories (category, work-memory type, scene) |
| `/compression` | CCR totals, recent records with savings %, and an interactive compress box |
| `/settings` | LLM endpoint / API key / model, with a live status card and a "Save & test" probe. The model is picked from the endpoint's own list (searchable dropdown); the embedding model is fixed and shown read-only. |
| `/captures` | Toggle prompt capture, browse the captured payloads and read them in full |
| `/model-map` | Model mappings the proxy applies: the model a client asks for → the model the upstream serves. Reads and writes the proxy's own config (`modelMappings`), the same file `imc config model-map` edits. |

## API routes

All routes shell out to the `imc` binary and return its `--json` payload verbatim.

- `POST /api/models` — saves the typed URL/key, then lists the endpoint's models
  (`imc config models` → `GET {base_url}/models`); returns `{url, models[]}`
- `GET /api/captures[?id=]` — captured prompts + capture state
- `POST /api/captures` — `{action: "on"|"off"|"clear"|"delete", id?}`
- `GET /api/config` — effective config with the key **masked** (the CLI masks it)
- `POST /api/config` — `{ base_url?, api_key?, model?, embedding_model?, test? }`; writes through
  `imc config set` and can probe the endpoint with `imc config test`
- `GET /api/projects[?root=]`
- `POST /api/search` — `{ query, root, limit?, noVector? }`
- `POST /api/trace` — `{ target, root, callers?, both?, depth? }`
- `GET|POST /api/memory` — list by `?root=`, add with `{ root, content, category, memType, scene }`
- `GET /api/compression?root=&limit=`
- `GET /api/compression/stats?root=`
- `POST /api/compress` — `{ text, root }`

## Running

From the CLI (starts the dashboard if needed, then opens it):

```bash
imc web                           # http://localhost:4789
imc web --url http://<host>:4789  # remote host
imc web --no-open                 # print the URL only
```

From source:

```bash
cd web-dashboard
npm install
npm run dev            # http://localhost:3000
# or
npm run build && npm run start -- -p 4789
```

## The data comes from the proxy

The dashboard does not shell out to the `imc` binary. It asks the proxy, over
HTTP, on `http://127.0.0.1:12345` (`IMC_PROXY_URL` to change it):

```
browser ──► dashboard /api/* ──► imc proxy :12345 ──► the index
```

The proxy is already running and already holds the state, so one request is one
HTTP call instead of one subprocess. The browser still talks to the dashboard's
own origin: the proxy holds the API keys and binds to localhost, and a
dashboard that is opened from another machine must not need the proxy exposed.

Start the proxy first:

```bash
imc proxy serve --port 12345   # the proxy, in the foreground
```

If it is not answering, the pages say so instead of spinning on "Loading".

## Configuration reminder

`llm.model` unset is the most common "imc does not work" cause: the built-in fallback model
is endpoint-specific and usually answers 404. The dashboard therefore:

- fetches the endpoint's model list into a dropdown (datalist) so the model name is
  never typed by hand — the key stays in the config file, the browser never sees it;
- shows a dismissible banner on every page while `llm.base_url` / `llm.model` / `llm.api_key`
  is incomplete, linking to `/settings`;
- shows an "LLM setup" card on the overview page;
- shows the missing keys on `/settings` itself.

The same check exists on the CLI: `imc doctor` lists `llm.model not set` as an issue, and
`imc search` warns before it sends a request that is likely to 404.

## Notes

- Project selection drives every action: each page picks a project and passes its
  filesystem path as `imc -C <path>`.
- The compression page reads the CCR store that belongs to the selected project.
- Search and trace results are rendered straight from the CLI contract, so a CLI change
  surfaces here without extra mapping.

## Run as a service (systemd)

`deploy/imc-dashboard.service` is a ready unit. Adjust `WorkingDirectory` and `ExecStart`
(node path), then:

```bash
sudo cp deploy/imc-dashboard.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now imc-dashboard
sudo ufw allow 4789/tcp          # if the host has ufw enabled
```

It listens on `0.0.0.0:4789` and restarts on failure.
