import { existsSync, readFileSync, writeFileSync, renameSync, mkdirSync } from "fs";
import { homedir } from "os";
import { dirname, join } from "path";

/**
 * The proxy's own configuration file.
 *
 * Python resolves it (main/proxy/lib/paths.py) in this order:
 *
 *   1. `<repo>/hy-pro/config.json`  — if it exists
 *   2. `<repo>/config.json`         — if it exists
 *   3. `$COPILOT_API_HOME/config.json` (default `~/.local/share/copilot-api/config.json`)
 *
 * The dashboard must write the same file the proxy reads, otherwise a mapping
 * made here would be ignored.
 */
export function proxyConfigPath(): string {
  const explicit = process.env.COPILOT_API_CONFIG_PATH;
  if (explicit && explicit.trim()) return explicit.trim();

  // The app lives in `<repo>/web-dashboard`, so the repo root is one level up.
  const repoRoot = join(process.cwd(), "..");
  for (const candidate of [
    join(repoRoot, "hy-pro", "config.json"),
    join(repoRoot, "config.json"),
  ]) {
    if (existsSync(candidate)) return candidate;
  }

  const home = process.env.COPILOT_API_HOME;
  const dir =
    home && home.trim() ? home.trim() : join(homedir(), ".local", "share", "copilot-api");
  return join(dir, "config.json");
}

export interface ModelMappings {
  /** Where the mapping lives. */
  path: string;
  /** model id the client asks for → model id the upstream serves. */
  mappings: Record<string, string>;
}

function readConfig(path: string): Record<string, unknown> {
  if (!existsSync(path)) return {};
  try {
    const parsed = JSON.parse(readFileSync(path, "utf-8"));
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

/** Write the config back, keeping every other key untouched. */
function writeConfig(path: string, config: Record<string, unknown>): void {
  mkdirSync(dirname(path), { recursive: true });
  // Write to a sibling file first, then rename: the proxy reads this file on
  // every request, so a half-written file would be seen as a broken config.
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(config, null, 2)}\n`, "utf-8");
  renameSync(tmp, path);
}

/** Read the current model mappings. */
export function readModelMappings(): ModelMappings {
  const path = proxyConfigPath();
  const config = readConfig(path);
  const raw = config.modelMappings;
  const mappings: Record<string, string> = {};
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    for (const [from, to] of Object.entries(raw as Record<string, unknown>)) {
      if (typeof to === "string") mappings[from] = to;
    }
  }
  return { path, mappings };
}

/**
 * Add or replace a mapping.
 *
 * The proxy strips a `provider:` prefix from the value before forwarding
 * ("hyperspace:gpt-5.6-luna" → "gpt-5.6-luna"), so the value is stored as
 * given.
 */
export function setModelMapping(from: string, to: string): ModelMappings {
  const path = proxyConfigPath();
  const config = readConfig(path);
  const existing =
    config.modelMappings && typeof config.modelMappings === "object" && !Array.isArray(config.modelMappings)
      ? { ...(config.modelMappings as Record<string, string>) }
      : {};
  existing[from] = to;
  config.modelMappings = existing;
  writeConfig(path, config);
  return readModelMappings();
}

/** Remove a mapping. Returns the state after the removal. */
export function removeModelMapping(from: string): ModelMappings {
  const path = proxyConfigPath();
  const config = readConfig(path);
  const existing =
    config.modelMappings && typeof config.modelMappings === "object" && !Array.isArray(config.modelMappings)
      ? { ...(config.modelMappings as Record<string, string>) }
      : {};
  delete existing[from];
  config.modelMappings = existing;
  writeConfig(path, config);
  return readModelMappings();
}
