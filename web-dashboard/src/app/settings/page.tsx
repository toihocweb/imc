"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Combobox } from "@/components/ui/combobox";
import { formatNumber } from "@/lib/format";

interface ConfigStatus {
  base_url: string;
  model: string;
  api_key_masked: string;
  api_key_set: boolean;
  embedding_model: string;
  config_path: string;
  complete: boolean;
  missing: string[];
}

export default function SettingsPage() {
  const [status, setStatus] = useState<ConfigStatus | null>(null);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [testOutput, setTestOutput] = useState<string | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [modelsUrl, setModelsUrl] = useState<string>("");
  const [fetchingModels, setFetchingModels] = useState(false);

  const load = async () => {
    try {
      const res = await fetch("/api/config");
      const d = await res.json();
      if (d.error) {
        setError(d.error);
        return;
      }
      setStatus(d as ConfigStatus);
      setBaseUrl(d.base_url || "");
      setModel(d.model || "");
    } catch (e: any) {
      setError(e.message);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const loadModels = async () => {
    setFetchingModels(true);
    setError(null);
    setMessage(null);
    try {
      const res = await fetch("/api/models", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          base_url: baseUrl || undefined,
          api_key: apiKey || undefined,
        }),
      });
      const d = await res.json();
      if (d.error) {
        setError(`Could not fetch models: ${d.error}`);
        setModels([]);
        return;
      }
      setModels(d.models || []);
      setModelsUrl(d.url || "");
      setMessage(`${formatNumber((d.models || []).length)} models fetched from ${d.url}`);
      if (!model && d.models && d.models.length) {
        setModel(d.models[0]);
      }
      load();
    } catch (e: any) {
      setError(e.message);
    } finally {
      setFetchingModels(false);
    }
  };

  const save = async (withTest: boolean) => {
    setBusy(true);
    setMessage(null);
    setError(null);
    setTestOutput(null);
    try {
      const res = await fetch("/api/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          base_url: baseUrl,
          model,
          api_key: apiKey || undefined,
          test: withTest,
        }),
      });
      const d = await res.json();
      if (d.errors && d.errors.length) setError(d.errors.join("; "));
      if (d.applied && d.applied.length) {
        setMessage(`Saved configuration: ${d.applied.join(", ")}`);
        setApiKey("");
      }
      if (d.test) setTestOutput(d.test);
      if (d.status) {
        setStatus(d.status);
        setBaseUrl(d.status.base_url || "");
        setModel(d.status.model || "");
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-6 w-full animate-in fade-in duration-300">
      <div>
        <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Configuration & Settings</h1>
        <p className="text-xs sm:text-sm text-muted-foreground mt-1">
          Configure upstream LLM endpoints, API credentials, and default model identifiers. Keys are masked and stored securely in local SQLite/config.
        </p>
      </div>

      {status && !status.complete && (
        <Card className="border-amber-500/60 bg-amber-500/10">
          <CardContent className="p-4">
            <div className="font-semibold text-xs sm:text-sm text-amber-700 dark:text-amber-400">
              Configuration Incomplete
            </div>
            <div className="text-xs text-muted-foreground mt-1">
              Missing fields: {status.missing.join(", ")}. Hybrid search continues to return lexical/vector chunks, but LLM reranking and automated reflection will be disabled until configured.
            </div>
          </CardContent>
        </Card>
      )}

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <div className="space-y-6 lg:col-span-2">
          <Card className="border-border/80 shadow-xs">
            <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
              <CardTitle className="text-sm font-semibold">Language Model Endpoint</CardTitle>
            </CardHeader>
            <CardContent className="p-5 space-y-4">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">Base URL</label>
                <Input
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="https://api.openai.com/v1"
                  className="text-xs font-mono"
                />
                <p className="text-[11px] text-muted-foreground">
                  OpenAI-compatible endpoint. `imc` proxies append /chat/completions and /embeddings automatically.
                </p>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">API Key</label>
                <div className="flex gap-2">
                  <Input
                    type={showKey ? "text" : "password"}
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder={status?.api_key_set ? status.api_key_masked : "sk-..."}
                    autoComplete="off"
                    className="text-xs font-mono flex-1"
                  />
                  <Button
                    variant="outline"
                    onClick={() => setShowKey((v) => !v)}
                    className="text-xs h-9 cursor-pointer"
                  >
                    {showKey ? "Hide" : "Show"}
                  </Button>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  {status?.api_key_set
                    ? "Key is already stored (masked above). Leave blank to keep existing key."
                    : "No API key configured. Enter a key to authenticate upstream calls."}
                </p>
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center justify-between gap-2">
                  <label className="text-xs font-medium text-foreground">Model Name</label>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={loadModels}
                    disabled={fetchingModels}
                    className="text-xs h-7 cursor-pointer"
                  >
                    {fetchingModels ? "Fetching..." : "Fetch Models"}
                  </Button>
                </div>
                <Combobox
                  id="imc-model"
                  value={model}
                  onChange={setModel}
                  options={models}
                  placeholder="claude-3-7-sonnet"
                  emptyLabel={
                    models.length === 0
                      ? "No models found — click Fetch Models"
                      : "No matching models"
                  }
                />
                <p className="text-[11px] text-muted-foreground">
                  {models.length > 0 ? (
                    <>
                      {formatNumber(models.length)} models available from {modelsUrl} — click the box to filter and select.
                    </>
                  ) : (
                    <>
                      Click &quot;Fetch Models&quot; to discover supported models from the endpoint.
                    </>
                  )}
                </p>
              </div>

              <div className="flex flex-wrap gap-2 pt-2">
                <Button onClick={() => save(false)} disabled={busy} className="text-xs h-8 cursor-pointer">
                  {busy ? "Saving..." : "Save Settings"}
                </Button>
                <Button variant="outline" onClick={() => save(true)} disabled={busy} className="text-xs h-8 cursor-pointer">
                  Save &amp; Test Connection
                </Button>
                <Button variant="ghost" onClick={load} disabled={busy} className="text-xs h-8 cursor-pointer">
                  Reload
                </Button>
              </div>

              {message && <div className="text-xs font-medium text-emerald-600 dark:text-emerald-400">{message}</div>}
              {error && <div className="text-xs font-medium text-destructive">{error}</div>}
              {testOutput && (
                <pre className="text-xs bg-muted/60 border border-border p-3 rounded-lg overflow-auto whitespace-pre-wrap font-mono">
                  {testOutput}
                </pre>
              )}
            </CardContent>
          </Card>
        </div>

        <div className="space-y-6">
          <Card className="border-border/80 shadow-xs">
            <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
              <CardTitle className="text-sm font-semibold">Active Engine Summary</CardTitle>
            </CardHeader>
        <CardContent className="p-5 space-y-2.5 text-xs">
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Config File</span>
            <span className="font-mono text-[11px] break-all text-right">{status?.config_path}</span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Base URL</span>
            <span className="font-mono text-[11px] break-all text-right">
              {status?.base_url || "—"}
            </span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Model Identifier</span>
            <span className="font-mono text-[11px] text-right">
              {status?.model || <span className="text-amber-600 dark:text-amber-400">not configured</span>}
            </span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">API Key</span>
            <span className="font-mono text-[11px] text-right">
              {status?.api_key_set ? status.api_key_masked : "—"}
            </span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Embedding Model</span>
            <span className="font-mono text-[11px] text-right">
              {status?.embedding_model || "text-embedding-3-small"}
              <span className="ml-2 text-[10px] uppercase text-muted-foreground font-sans">fixed</span>
            </span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">LLM Operational Readiness</span>
            {status?.complete ? (
              <Badge className="bg-emerald-500/15 text-emerald-500 border-emerald-500/20 text-[10px]">
                Ready
              </Badge>
            ) : (
              <Badge variant="destructive" className="text-[10px]">Incomplete</Badge>
            )}
          </div>
          <p className="text-[11px] text-muted-foreground pt-1">
            This configuration is shared across the CLI, MCP server, and proxy. Test from terminal with{" "}
            <code className="font-mono bg-muted px-1.5 py-0.5 rounded">imc doctor</code> or{" "}
            <code className="font-mono bg-muted px-1.5 py-0.5 rounded">imc config test</code>.
          </p>
        </CardContent>
      </Card>
        </div>
      </div>
    </div>
  );
}
