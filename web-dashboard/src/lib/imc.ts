import { spawn, type ChildProcessWithoutNullStreams } from "child_process";

const IMC_BINARY =
  process.env.IMC_BIN ||
  "/root/imc-rs/target/release/imc";

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

function runImc(
  args: string[],
  options: { root?: string; input?: string } = {}
): Promise<string> {
  return new Promise((resolve, reject) => {
    const env: NodeJS.ProcessEnv = { ...process.env };
    delete env.IMC_DATA_DIR;
    delete env.IMC_CONFIG;
    delete env.IMC_VECTOR;
    delete env.IMC_EMBED_API_KEY;
    delete env.OPENAI_API_KEY;

    const child = spawn(IMC_BINARY, args, {
      env,
      cwd: options.root,
      stdio: ["pipe", "pipe", "pipe"],
    }) as ChildProcessWithoutNullStreams;

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data: Buffer) => {
      stdout += data.toString();
    });
    child.stderr.on("data", (data: Buffer) => {
      stderr += data.toString();
    });

    child.on("close", (code: number | null) => {
      if (code !== 0) {
        reject(new Error(stderr || `Command failed with code ${code}`));
      } else {
        resolve(stdout);
      }
    });

    if (options.input) {
      child.stdin.write(options.input);
    }
    child.stdin.end();
  });
}

export async function listProjects(root?: string): Promise<Project[]> {
  const stdout = await runImc(["-C", root || ".", "list", "--json"], { root });
  return JSON.parse(stdout);
}

export async function search(
  query: string,
  options: { root?: string; limit?: number; noVector?: boolean } = {}
): Promise<SearchHit[]> {
  const args = ["search", query, "--json", "-l", String(options.limit || 10)];
  if (options.noVector) args.push("--no-vector");
  const stdout = await runImc(["-C", options.root || ".", ...args], {
    root: options.root,
  });
  return JSON.parse(stdout);
}

export async function trace(
  target: string,
  options: { root?: string; callers?: boolean; both?: boolean; depth?: number } = {}
): Promise<TraceResult> {
  const args = ["trace", target, "--json"];
  if (options.callers) args.push("--callers");
  if (options.both) args.push("--both");
  if (options.depth) args.push("-d", String(options.depth));
  const stdout = await runImc(["-C", options.root || ".", ...args], {
    root: options.root,
  });
  return JSON.parse(stdout);
}

export async function memoryList(root?: string, limit: number = 50): Promise<MemoryEntry[]> {
  const stdout = await runImc(
    ["-C", root || ".", "memory-list", "--json", "-l", String(limit)],
    { root }
  );
  return JSON.parse(stdout);
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
  const args = [
    "memory-add",
    content,
    "--category",
    category,
    "--type",
    memType,
    "--scene",
    scene,
    "--importance",
    String(importance),
  ];
  for (const p of paths) args.push("-p", p);
  for (const t of tags) args.push("-t", t);
  const stdout = await runImc(["-C", root, ...args], { root });
  return stdout;
}

export async function ccrStats(root?: string): Promise<CcrStats> {
  const stdout = await runImc(["-C", root || ".", "ccr", "stats", "--json"], { root });
  return JSON.parse(stdout);
}

export async function ccrList(root?: string, limit: number = 20): Promise<CcrEntry[]> {
  const stdout = await runImc(
    ["-C", root || ".", "ccr", "list", "--json", "-l", String(limit)],
    { root }
  );
  return JSON.parse(stdout);
}

export async function compressText(text: string, root?: string): Promise<string> {
  const stdout = await runImc(["-C", root || ".", "compress", "--raw"], {
    root,
    input: text,
  });
  return stdout;
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
  const raw = JSON.parse(
    await runImc(["--json", "config", "list"])
  ) as Record<string, string | null>;
  const path = (await runImc(["config", "path"])).trim();

  const base_url = raw["llm.base_url"] || "";
  const model = raw["llm.model"] || "";
  const api_key_masked = raw["llm.api_key"] || "";
  const embedding_model = raw["embedding.model"] || "";

  const missing: string[] = [];
  if (!base_url) missing.push("llm.base_url");
  if (!model) missing.push("llm.model");
  if (!api_key_masked) missing.push("llm.api_key");

  return {
    base_url,
    model,
    api_key_masked,
    api_key_set: api_key_masked.length > 0,
    embedding_model,
    config_path: path,
    complete: missing.length === 0,
    missing,
  };
}

/** Write one dotted config key (`llm.base_url`, `llm.model`, ...). */
export async function configSet(key: string, value: string): Promise<string> {
  return runImc(["config", "set", key, value]);
}

/** Probe the configured endpoint. Returns the CLI's report as text. */
export async function configTest(): Promise<string> {
  return runImc(["config", "test"]);
}

export interface ModelList {
  url: string;
  /** Chat models (the default selection of `imc config models`). */
  models: string[];
  chat_models?: string[];
  embedding_models?: string[];
}

/**
 * Ask the configured endpoint which models it serves (`GET {base_url}/models`).
 * The key stays in the config file — the CLI sends it, the browser never sees it.
 */
export async function fetchModels(): Promise<ModelList> {
  const raw = JSON.parse(await runImc(["--json", "config", "models"]));
  return {
    url: raw.url || "",
    models: Array.isArray(raw.models) ? raw.models : [],
    chat_models: Array.isArray(raw.chat_models) ? raw.chat_models : undefined,
    embedding_models: Array.isArray(raw.embedding_models) ? raw.embedding_models : undefined,
  };
}
