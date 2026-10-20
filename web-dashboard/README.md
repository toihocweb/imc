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

## API routes

All routes shell out to the `imc` binary and return its `--json` payload verbatim.

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

The binary is resolved from `IMC_BIN`, defaulting to `/root/imc-rs/target/release/imc`.
Point it elsewhere when imc is installed system-wide:

```bash
IMC_BIN=$(command -v imc) npm run start -- -p 4789
```

Every call unsets `IMC_DATA_DIR`, `IMC_CONFIG`, `IMC_VECTOR`, `IMC_EMBED_API_KEY` and
`OPENAI_API_KEY` so the dashboard never accidentally targets a different project or
calls a real embedding endpoint.

## Notes

- Project selection drives every action: each page picks a project and passes its
  filesystem path as `imc -C <path>`.
- The compression page reads the CCR store that belongs to the selected project.
- Search and trace results are rendered straight from the CLI contract, so a CLI change
  surfaces here without extra mapping.

## Run as a service (systemd)

`deploy/imc-dashboard.service` is a ready unit. Adjust `WorkingDirectory`, `ExecStart`
(node path) and `IMC_BIN`, then:

```bash
sudo cp deploy/imc-dashboard.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now imc-dashboard
sudo ufw allow 4789/tcp          # if the host has ufw enabled
```

It listens on `0.0.0.0:4789` and restarts on failure.
