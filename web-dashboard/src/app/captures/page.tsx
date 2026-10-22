"use client";

import { useEffect, useState } from "react";
import { Terminal, Trash2, Power, Eye, X, Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { formatNumber, formatBytes } from "@/lib/format";

interface CaptureInfo {
  id: string;
  path: string;
  size: number;
  modified: number;
  model: string;
  preview: string;
}

interface CaptureStatus {
  enabled: boolean;
  captures: number;
  dir: string;
}

export default function CapturesPage() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [captures, setCaptures] = useState<CaptureInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [payload, setPayload] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const load = async () => {
    try {
      const res = await fetch("/api/captures");
      const d = await res.json();
      if (d.ok) {
        setStatus(d.status);
        setCaptures(d.captures || []);
      } else {
        setError(d.error);
      }
    } catch (e: any) {
      setError(e.message);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const act = async (body: any) => {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/captures", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const d = await res.json();
      if (d.ok) {
        setStatus(d.status);
        setCaptures(d.captures || []);
      } else {
        setError(d.error);
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const view = async (id: string) => {
    setSelected(id);
    setPayload(null);
    try {
      const res = await fetch(`/api/captures?id=${encodeURIComponent(id)}`);
      const d = await res.json();
      setPayload(d.ok ? JSON.stringify(d.payload, null, 2) : String(d.error));
    } catch (e: any) {
      setPayload(`error: ${e.message}`);
    }
  };

  const copyPayload = () => {
    if (!payload) return;
    navigator.clipboard.writeText(payload);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const fmtTime = (sec: number) =>
    sec ? new Date(sec * 1000).toLocaleString("en-US", { hour12: false }) : "—";

  return (
    <div className="space-y-6 w-full animate-in fade-in duration-300">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight sm:text-3xl flex items-center gap-2">
            <Terminal className="h-6 w-6 text-primary" /> Prompt Captures
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Raw payload logging for upstream LLM requests: system prompt, tool schemas, memory injections, and tool results.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant={status?.enabled ? "destructive" : "default"}
            onClick={() => act({ action: status?.enabled ? "off" : "on" })}
            disabled={busy}
            className="text-xs h-8 cursor-pointer gap-1.5"
          >
            <Power className="h-3.5 w-3.5" />
            {status?.enabled ? "Disable Capture" : "Enable Capture"}
          </Button>
          <Button
            variant="outline"
            onClick={() => act({ action: "clear" })}
            disabled={busy || captures.length === 0}
            className="text-xs h-8 cursor-pointer gap-1.5"
          >
            <Trash2 className="h-3.5 w-3.5" />
            Clear All
          </Button>
        </div>
      </div>

      {error && (
        <Card className="border-destructive/40 bg-destructive/10 text-destructive">
          <CardContent className="p-4 text-xs font-medium">{error}</CardContent>
        </Card>
      )}

      <Card className="border-border/80 shadow-xs">
        <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
          <CardTitle className="text-sm font-semibold flex items-center justify-between">
            <span>Capture Engine Status</span>
            {status?.enabled ? (
              <Badge className="bg-emerald-500/15 text-emerald-500 border-emerald-500/20">
                Active (Recording)
              </Badge>
            ) : (
              <Badge variant="secondary">Disabled</Badge>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="p-5 space-y-2 text-xs">
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Recorded Payloads</span>
            <span className="font-mono font-semibold text-foreground">
              {formatNumber(status?.captures ?? captures.length)} entries
            </span>
          </div>
          <div className="flex justify-between gap-4 border-b border-border/40 pb-2">
            <span className="text-muted-foreground">Storage Directory</span>
            <span className="font-mono text-muted-foreground break-all text-right">{status?.dir}</span>
          </div>
          <p className="text-[11px] text-muted-foreground pt-1">
            Equivalent CLI commands: <code className="font-mono bg-muted px-1.5 py-0.5 rounded">imc capture on|off|list|show|rm|clear</code>.
            Takes effect immediately without restarting the proxy.
          </p>
        </CardContent>
      </Card>

      <Card className="border-border/80 shadow-xs">
        <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
          <CardTitle className="text-sm font-semibold flex items-center justify-between">
            <span>Captured Payloads ({formatNumber(captures.length)})</span>
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0 overflow-x-auto scroll-thin">
          {captures.length === 0 ? (
            <div className="p-8 text-center text-xs text-muted-foreground">
              No captures recorded yet. Enable capture and send a request through the proxy to inspect payloads.
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow className="text-xs">
                  <TableHead className="w-24">ID</TableHead>
                  <TableHead>Timestamp</TableHead>
                  <TableHead>Model</TableHead>
                  <TableHead className="text-right">Payload Size</TableHead>
                  <TableHead>Preview</TableHead>
                  <TableHead className="text-right w-24">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody className="text-xs">
                {captures.map((c) => (
                  <TableRow key={c.id} className="hover:bg-muted/40 transition-colors">
                    <TableCell className="font-mono font-medium text-foreground">#{c.id}</TableCell>
                    <TableCell className="whitespace-nowrap font-mono text-muted-foreground text-[11px]">
                      {fmtTime(c.modified)}
                    </TableCell>
                    <TableCell className="font-mono">{c.model || "—"}</TableCell>
                    <TableCell className="whitespace-nowrap text-right font-mono">
                      {formatBytes(c.size)}
                    </TableCell>
                    <TableCell className="max-w-[280px] truncate text-muted-foreground font-mono text-[11px]">
                      {c.preview || "—"}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => view(c.id)}
                          className="h-7 px-2 text-xs cursor-pointer"
                        >
                          <Eye className="h-3.5 w-3.5 mr-1" /> View
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => act({ action: "delete", id: c.id })}
                          disabled={busy}
                          className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive cursor-pointer"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {selected && (
        <Card className="border-border/80 shadow-xs">
          <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
            <CardTitle className="text-sm font-semibold flex items-center justify-between">
              <span className="font-mono">Payload Inspection #{selected}</span>
              <div className="flex items-center gap-2">
                {payload && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={copyPayload}
                    className="h-7 text-xs gap-1 cursor-pointer"
                  >
                    {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                    {copied ? "Copied" : "Copy JSON"}
                  </Button>
                )}
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setSelected(null)}
                  className="h-7 w-7 p-0 cursor-pointer"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </CardTitle>
          </CardHeader>
          <CardContent className="p-4">
            {payload ? (
              <pre className="max-h-[65vh] overflow-auto whitespace-pre-wrap rounded-lg bg-background/60 border border-border p-4 font-mono text-xs leading-relaxed text-foreground scroll-thin">
                {payload}
              </pre>
            ) : (
              <p className="text-xs text-muted-foreground italic">Loading payload content...</p>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
