/**
 * The dashboard's data comes from the imc proxy, not from a subprocess.
 *
 * It used to shell out to the `imc` binary for every request: one process
 * per click, and a rebuild in progress was a dead page ("Loading projects..."
 * for ever). The proxy is already running and already holds the state, so the
 * data is asked for over HTTP. The dashboard's own `/api/*` routes stay where
 * they are — the browser talks to the same origin, the server talks to the
 * proxy — so nothing is exposed publicly and no CORS dance is needed.
 */
const PROXY_URL = (
  process.env.IMC_PROXY_URL || "http://127.0.0.1:35711"
).replace(/\/$/, "");

async function proxy<T = unknown>(
  path: string,
  init?: RequestInit
): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${PROXY_URL}${path}`, {
      ...init,
      headers: {
        "content-type": "application/json",
        ...(init?.headers || {}),
      },
    });
  } catch (e: any) {
    throw new Error(
      `the imc proxy is not answering on ${PROXY_URL} — start it with \`imc proxy serve --port 35711\` (${e?.message ?? e})`
    );
  }
  const text = await res.text();
  let body: any = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = text;
  }
  if (!res.ok) {
    throw new Error(
      (body && typeof body === "object" && body.error) ||
        `the imc proxy answered ${res.status}`
    );
  }
  return body as T;
}

export interface ProjectCounts {
  files: number;
  symbols: number;
  edges: number;
  chunks: number;
  embeddings: number;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  counts: ProjectCounts;
  db_size_mb: number;
}

export interface SearchHit {
  score: number;
  file: string;
  start_line: number;
  end_line: number;
  text: string;
  snippet: string;
  sym_name?: string;
  sym_kind?: string;
  source: string;
}

export interface CcrStats {
  db: string;
  records: number;
  original_tokens: number;
  compressed_tokens: number;
  saved_tokens: number;
  compression_ratio: number;
  db_bytes: number;
}

export interface CcrEntry {
  id: string;
  kind: string;
  original?: string;
  compressed?: string;
  orig_tokens: number;
  comp_tokens: number;
  created_at: number;
}

export interface MemoryEntry {
  id: number;
  content: string;
  mem_type: string;
  scene: string;
  category: string;
  paths: string[];
  tags: string[];
  importance: number;
  use_count: number;
  created_at: number;
  last_used: number;
  source: string;
}

export interface TraceNode {
  id: string;
  name: string;
  file: string;
  line: number;
  type: string;
  callees?: TraceNode[];
  callers?: TraceNode[];
  depth: number;
}

export interface TraceArm {
  mode: string;
  tree: TraceNode;
}

export interface TraceResult {
  project: string;
  target: string;
  depth: number;
  results: TraceArm[];
}

export async function listProjects(root?: string): Promise<Project[]> {
  const projects = await proxy<Project[]>("/api/projects");
  if (!root) return projects;
  return projects.filter((p) => p.path === root);
}

export async function search(
  query: string,
  options: { root?: string; limit?: number; noVector?: boolean } = {}
): Promise<SearchHit[]> {
  return proxy<SearchHit[]>("/api/search", {
    method: "POST",
    body: JSON.stringify({
      query,
      root: options.root,
      limit: options.limit || 10,
      noVector: options.noVector,
    }),
  });
}

export async function trace(
  target: string,
  options: { root?: string; callers?: boolean; both?: boolean; depth?: number } = {}
): Promise<TraceResult> {
  return proxy<TraceResult>("/api/trace", {
    method: "POST",
    body: JSON.stringify({
      target,
      root: options.root,
      callers: options.callers,
      both: options.both,
      depth: options.depth,
    }),
  });
}

export async function memoryList(root?: string, limit: number = 50): Promise<MemoryEntry[]> {
  const q = new URLSearchParams();
  if (root) q.set("root", root);
  q.set("limit", String(limit));
  return proxy<MemoryEntry[]>(`/api/memory?${q.toString()}`);
}

export async function memoryAdd(
  root: string,
  content: string,
  category: string = "note",
  memType: string = "note",
  scene: string = "",
  paths: string[] = [],
  tags: string[] = [],
  importance: number = 0.5
) {
  const result = await proxy<{ id: number | null; stored: boolean }>(
    "/api/memory",
    {
      method: "POST",
      body: JSON.stringify({
        root,
        content,
        category,
        mem_type: memType,
        scene,
        paths,
        tags,
        importance,
      }),
    }
  );
  return result.stored ? `stored memory #${result.id}` : "not stored";
}

export async function memoryDelete(
  id: number,
  root?: string
): Promise<{ ok: boolean; deleted: boolean; id: number }> {
  const q = new URLSearchParams({ id: String(id) });
  if (root) q.set("root", root);
  return proxy<{ ok: boolean; deleted: boolean; id: number }>(`/api/memory?${q.toString()}`, {
    method: "DELETE",
  });
}

export async function ccrStats(root?: string): Promise<CcrStats> {
  return proxy<CcrStats>("/api/compression/stats");
}

export async function ccrList(root?: string, limit: number = 20, id?: string): Promise<CcrEntry[] | CcrEntry> {
  const q = new URLSearchParams({ limit: String(limit) });
  if (id) q.set("id", id);
  return proxy<CcrEntry[] | CcrEntry>(`/api/compression?${q.toString()}`);
}

export async function compressText(text: string, root?: string): Promise<string> {
  const result = await proxy<{ output: string }>("/api/compress", {
    method: "POST",
    body: JSON.stringify({ text }),
  });
  return result.output;
}

export interface ConfigStatus {
  base_url: string;
  model: string;
  api_key_masked: string;
  api_key_set: boolean;
  embedding_model: string;
  config_path: string;
  /** True when base_url, model and api_key are all present. */
  complete: boolean;
  /** Which of the LLM settings are missing. */
  missing: string[];
}

/** Read the effective imc config (secrets stay masked: the CLI masks them). */
export async function configStatus(): Promise<ConfigStatus> {
  const raw = await proxy<ConfigStatus>("/api/config");
  return raw;
}

/** Write one dotted config key (`llm.base_url`, `llm.model`, ...). */
export async function configSet(key: string, value: string): Promise<string> {
  await proxy("/api/config", {
    method: "POST",
    body: JSON.stringify({ key, value }),
  });
  return `${key} saved`;
}

/** Probe the configured endpoint. Returns the CLI's report as text. */
export async function configTest(): Promise<string> {
  const result = await proxy<{ ok: boolean; output: string }>("/api/config/test");
  return result.output;
}

export interface ModelList {
  url: string;
  /** Chat models (the default selection of `imc config models`). */
  models: string[];
  chat_models?: string[];
  embedding_models?: string[];
}

export interface UsageRecord {
  timestamp: string;
  model: string;
  endpoint: string;
  usage?: {
    input_tokens?: number;
    output_tokens?: number;
    prompt_tokens?: number;
    completion_tokens?: number;
    cache_read_input_tokens?: number;
    cache_creation_input_tokens?: number;
  };
  copilot_usage?: any;
}

export interface ModelUsageSummary {
  requests: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
}

export interface UsageData {
  file: string;
  total_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  cache_hit_rate: number;
  by_model: Record<string, ModelUsageSummary>;
  records: UsageRecord[];
}

export async function getUsage(): Promise<UsageData> {
  return proxy<UsageData>("/api/usage");
}

export async function clearUsage(): Promise<void> {
  await fetch("/api/usage", { method: "DELETE" });
}

/**
 * Ask the configured endpoint which models it serves (`GET {base_url}/models`).
 * The key stays in the config file — the CLI sends it, the browser never sees it.
 */
export async function fetchModels(): Promise<ModelList> {
  const raw = await proxy<ModelList>("/api/models");
  return {
    url: raw.url || "",
    models: Array.isArray(raw.models) ? raw.models : [],
    chat_models: Array.isArray(raw.chat_models) ? raw.chat_models : undefined,
    embedding_models: Array.isArray(raw.embedding_models) ? raw.embedding_models : undefined,
  };
}
