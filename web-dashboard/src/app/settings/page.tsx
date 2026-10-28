"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Key, Server, CheckCircle2, AlertCircle, Eye, EyeOff, Loader2 } from "lucide-react";
import { useConnection } from "@/components/connection-gate";

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
  const { secret: gateSecret, disconnect } = useConnection();
  const [status, setStatus] = useState<ConfigStatus | null>(null);
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [testOutput, setTestOutput] = useState<string | null>(null);

  const load = async () => {
    try {
      const res = await fetch("/api/config");
      const d = await res.json();
      if (d.error) {
        setError(d.error);
        return;
      }
      setStatus(d as ConfigStatus);
    } catch (e: any) {
      setError(e.message);
    }
  };

  useEffect(() => {
    load();
    const saved = localStorage.getItem("imc_hyperspace_secret") || gateSecret;
    if (saved) {
      setApiKey(saved);
    }
  }, [gateSecret]);

  const handleSaveSecret = async () => {
    if (!apiKey.trim()) {
      setError("Please enter an API key or secret.");
      return;
    }
    setBusy(true);
    setMessage(null);
    setError(null);
    setTestOutput(null);
    try {
      const res = await fetch("/api/connection/check", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ secret: apiKey.trim() }),
      });
      const data = await res.json();
      if (res.ok && data.ok) {
        localStorage.setItem("imc_hyperspace_secret", apiKey.trim());
        setMessage(data.message || "Secret saved and Hyperspace connection verified!");
        setTestOutput(`Target: ${data.endpoint || "http://127.0.0.1:6655"}\nStatus: HTTP ${data.status || 200} OK`);
        await load();
      } else {
        setError(data.error || "Failed to verify connection to Hyperspace on port 6655.");
      }
    } catch (e: any) {
      setError(e.message || "Failed to verify secret.");
    } finally {
      setBusy(false);
    }
  };

  const handleTestConnection = async () => {
    const keyToTest = apiKey.trim() || status?.api_key_masked;
    if (!keyToTest) {
      setError("Please enter an API key to test.");
      return;
    }
    setBusy(true);
    setMessage(null);
    setError(null);
    setTestOutput(null);
    try {
      const res = await fetch("/api/connection/check", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ secret: keyToTest }),
      });
      const data = await res.json();
      if (res.ok && data.ok) {
        setMessage(data.message || "Hyperspace connection verified successfully!");
        setTestOutput(`Target: ${data.endpoint || "http://127.0.0.1:6655"}\nStatus: HTTP ${data.status || 200} OK\nResponse: Connection healthy`);
      } else {
        setError(data.error || "Connection test failed.");
        if (data.error) setTestOutput(data.error);
      }
    } catch (e: any) {
      setError(e.message || "Connection test request failed.");
    } finally {
      setBusy(false);
    }
  };

  const handleClearSecret = async () => {
    disconnect();
    setApiKey("");
    setMessage("Secret removed from browser storage.");
    setError(null);
    setTestOutput(null);
  };

  return (
    <div className="w-full space-y-6 animate-in fade-in duration-200">
      <div>
        <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Configuration & Settings</h1>
        <p className="text-xs sm:text-sm text-muted-foreground mt-1">
          Manage Hyperspace connection credentials and inspect runtime engine settings.
        </p>
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-3">
        <div className="space-y-6 lg:col-span-2">
          <Card className="border-border/80 shadow-xs">
            <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Server className="h-4 w-4 text-primary" /> Hyperspace Connection
              </CardTitle>
            </CardHeader>
            <CardContent className="p-5 space-y-4">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">Target Endpoint</label>
                <div className="flex items-center gap-2">
                  <Input
                    readOnly
                    value="http://localhost:6655/anthropic"
                    className="text-xs font-mono bg-muted/40 text-muted-foreground flex-1"
                  />
                  <Badge variant="secondary" className="font-mono text-[10px] px-2.5 py-1">
                    Port 6655 (Default)
                  </Badge>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  Hyperspace upstream service listening on port 6655. All Claude Code proxy traffic automatically routes here.
                </p>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground flex items-center gap-1.5">
                  <Key className="h-3.5 w-3.5 text-primary" /> API Key / Secret
                </label>
                <div className="flex gap-2">
                  <Input
                    type={showKey ? "text" : "password"}
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder={status?.api_key_set ? status.api_key_masked : "Enter Hyperspace secret..."}
                    autoComplete="off"
                    className="text-xs font-mono flex-1"
                  />
                  <Button
                    variant="outline"
                    onClick={() => setShowKey((v) => !v)}
                    className="text-xs h-9 cursor-pointer"
                  >
                    {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                  </Button>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  {status?.api_key_set
                    ? "Secret is configured in IMC settings. Update here to re-verify and refresh."
                    : "Enter your Hyperspace secret key to authenticate upstream proxy requests."}
                </p>
              </div>

              <div className="flex flex-wrap gap-2 pt-2">
                <Button
                  onClick={handleSaveSecret}
                  disabled={busy || !apiKey.trim()}
                  className="text-xs h-8 cursor-pointer"
                >
                  {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" /> : null}
                  Save Secret
                </Button>
                <Button
                  variant="outline"
                  onClick={handleTestConnection}
                  disabled={busy}
                  className="text-xs h-8 cursor-pointer"
                >
                  {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" /> : null}
                  Test Connection
                </Button>
                <Button
                  variant="ghost"
                  onClick={handleClearSecret}
                  disabled={busy}
                  className="text-xs h-8 cursor-pointer text-muted-foreground hover:text-destructive"
                >
                  Clear Secret
                </Button>
              </div>

              {message && (
                <div className="text-xs font-medium text-emerald-600 dark:text-emerald-400 flex items-center gap-1.5">
                  <CheckCircle2 className="h-4 w-4 shrink-0" /> {message}
                </div>
              )}
              {error && (
                <div className="text-xs font-medium text-destructive flex items-center gap-1.5">
                  <AlertCircle className="h-4 w-4 shrink-0" /> {error}
                </div>
              )}

              {testOutput && (
                <pre className="text-xs bg-muted/60 border border-border p-3 rounded-lg overflow-auto whitespace-pre-wrap font-mono text-muted-foreground">
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
            <CardContent className="p-5 space-y-3 text-xs">
              <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
                <span className="text-muted-foreground">Config File</span>
                <span className="font-mono text-[11px] break-all text-right">{status?.config_path || "~/.imc_config.yaml"}</span>
              </div>
              <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
                <span className="text-muted-foreground">Upstream Target</span>
                <span className="font-mono text-[11px] break-all text-right">
                  http://localhost:6655
                </span>
              </div>
              <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
                <span className="text-muted-foreground">Secret Status</span>
                <span className="font-mono text-[11px] text-right">
                  {status?.api_key_set ? status.api_key_masked : (apiKey ? "Entered" : "—")}
                </span>
              </div>
              <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
                <span className="text-muted-foreground">Operational Status</span>
                {status?.api_key_set || apiKey ? (
                  <Badge className="bg-emerald-500/15 text-emerald-500 border-emerald-500/20 text-[10px]">
                    Connected
                  </Badge>
                ) : (
                  <Badge variant="outline" className="text-muted-foreground text-[10px]">
                    Needs Secret
                  </Badge>
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
