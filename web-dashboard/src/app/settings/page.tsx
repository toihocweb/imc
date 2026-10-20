"use client";
import { useEffect, useState } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";

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
  const [embeddingModel, setEmbeddingModel] = useState("");
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
      setEmbeddingModel(d.embedding_model || "");
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
        setError(`could not fetch models: ${d.error}`);
        setModels([]);
        return;
      }
      setModels(d.models || []);
      setModelsUrl(d.url || "");
      setMessage(`${(d.models || []).length} models from ${d.url}`);
      if (!model && d.models && d.models.length) {
        setModel(d.models[0]);
      }
      // the endpoint may have been new: refresh what we know
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
          embedding_model: embeddingModel,
          api_key: apiKey || undefined,
          test: withTest,
        }),
      });
      const d = await res.json();
      if (d.errors && d.errors.length) setError(d.errors.join("; "));
      if (d.applied && d.applied.length) {
        setMessage(`saved: ${d.applied.join(", ")}`);
        setApiKey("");
      }
      if (d.test) setTestOutput(d.test);
      if (d.status) {
        setStatus(d.status);
        setBaseUrl(d.status.base_url || "");
        setModel(d.status.model || "");
        setEmbeddingModel(d.status.embedding_model || "");
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Settings</h1>
        <p className="text-sm text-muted-foreground">
          LLM endpoint, key and model. The key is never sent back to the browser — the CLI
          masks it, and only a new value written here replaces it.
        </p>
      </div>

      {status && !status.complete && (
        <Card className="border-amber-500/60 bg-amber-500/10">
          <CardContent className="pt-6">
            <div className="font-medium">Model chưa cấu hình xong</div>
            <div className="text-sm text-muted-foreground mt-1">
              Thiếu: {status.missing.join(", ")}. Cho tới khi đủ, `imc search` chỉ trả
              raw hits (không có câu trả lời từ LLM) và các lệnh cần LLM sẽ lỗi.
            </div>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">LLM</CardTitle>
        </CardHeader>
        <CardContent className="space-y-5">
          <div className="space-y-2">
            <label className="text-sm font-medium">Base URL</label>
            <Input
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="https://api.example.com/v1"
            />
            <p className="text-xs text-muted-foreground">
              OpenAI-compatible endpoint. `imc` appends /chat/completions and /embeddings.
            </p>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium">API key</label>
            <div className="flex gap-2">
              <Input
                type={showKey ? "text" : "password"}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={status?.api_key_set ? status.api_key_masked : "sk-..."}
                autoComplete="off"
              />
              <Button variant="outline" onClick={() => setShowKey((v) => !v)}>
                {showKey ? "Hide" : "Show"}
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              {status?.api_key_set
                ? "Key hiện tại đã lưu (hiển thị dạng che). Để trống = giữ nguyên key cũ."
                : "Chưa có key. Nhập để lưu."}
            </p>
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between gap-2">
              <label className="text-sm font-medium">Model</label>
              <Button
                variant="outline"
                size="sm"
                onClick={loadModels}
                disabled={fetchingModels}
              >
                {fetchingModels ? "Fetching..." : "Fetch models"}
              </Button>
            </div>
            <Input
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="claude-haiku-4.5"
              list="imc-model-list"
            />
            <datalist id="imc-model-list">
              {models.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
            <p className="text-xs text-muted-foreground">
              {models.length > 0 ? (
                <>
                  {models.length} model từ {modelsUrl} — bấm vào ô để xem danh sách và lọc.
                </>
              ) : (
                <>
                  Bấm &quot;Fetch models&quot; để lấy danh sách model từ endpoint (URL + key
                  được lưu trước rồi mới gọi). Thiếu model là nguyên nhân phổ biến nhất của
                  lỗi 404 &quot;model is not available&quot;.
                </>
              )}
            </p>
          </div>

          <Separator />

          <div className="space-y-2">
            <label className="text-sm font-medium">Embedding model</label>
            <Input
              value={embeddingModel}
              onChange={(e) => setEmbeddingModel(e.target.value)}
              placeholder="text-embedding-3-small"
            />
            <p className="text-xs text-muted-foreground">
              Dùng cho chân vector của search. Đổi model này cần index lại (`imc . --force`).
            </p>
          </div>

          <div className="flex flex-wrap gap-2">
            <Button onClick={() => save(false)} disabled={busy}>
              {busy ? "Saving..." : "Save"}
            </Button>
            <Button variant="outline" onClick={() => save(true)} disabled={busy}>
              Save &amp; test
            </Button>
            <Button variant="ghost" onClick={load} disabled={busy}>
              Reload
            </Button>
          </div>

          {message && <div className="text-sm text-green-600 dark:text-green-400">{message}</div>}
          {error && <div className="text-sm text-destructive">{error}</div>}
          {testOutput && (
            <pre className="text-xs bg-muted p-3 rounded overflow-auto whitespace-pre-wrap">
              {testOutput}
            </pre>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Status</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-sm">
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Config file</span>
            <span className="font-mono text-xs break-all text-right">{status?.config_path}</span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Base URL</span>
            <span className="font-mono text-xs break-all text-right">
              {status?.base_url || "—"}
            </span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Model</span>
            <span className="font-mono text-xs text-right">
              {status?.model || <span className="text-amber-600 dark:text-amber-400">not set</span>}
            </span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">API key</span>
            <span className="font-mono text-xs text-right">
              {status?.api_key_set ? status.api_key_masked : "—"}
            </span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Embedding model</span>
            <span className="font-mono text-xs text-right">{status?.embedding_model || "—"}</span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Ready for LLM answers</span>
            {status?.complete ? (
              <Badge>yes</Badge>
            ) : (
              <Badge variant="destructive">no</Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground pt-2">
            Cùng cấu hình này dùng cho CLI, MCP server và proxy. Kiểm tra từ terminal:{" "}
            <code className="font-mono">imc doctor</code> hoặc{" "}
            <code className="font-mono">imc config test</code>.{" "}
            <Link href="/search" className="text-primary hover:underline">
              Thử search
            </Link>
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
