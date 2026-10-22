"use client";

import { useEffect, useState } from "react";
import {
  Cpu,
  Eye,
  Copy,
  Check,
  Sparkles,
  Database,
  Layers,
  FileCode,
  FileDiff,
  Terminal,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useProject } from "@/components/project-context";

interface CcrStats {
  db: string;
  records: number;
  original_tokens: number;
  compressed_tokens: number;
  saved_tokens: number;
  compression_ratio: number;
  db_bytes: number;
}

interface CcrEntry {
  id: string;
  kind: string;
  original?: string;
  compressed?: string;
  orig_tokens: number;
  comp_tokens: number;
  created_at: number;
}

const PRESETS = [
  {
    name: "Cargo Build Log",
    icon: Terminal,
    text: `Compiling serde v1.0.210
Compiling tokio v1.40.0
Compiling axum v0.7.5
error[E0425]: cannot find value \`undefined_variable\` in this scope
  --> src/main.rs:18:5
   |
18 |     undefined_variable.run();
   |     ^^^^^^^^^^^^^^^^^^ not found in this scope
For more information about this error, try \`rustc --explain E0425\`.
error: could not compile \`my_project\` (bin \`my_project\`) due to 1 previous error`,
  },
  {
    name: "Git Diff",
    icon: FileDiff,
    text: `diff --git a/src/auth.ts b/src/auth.ts
index e69de29..d95f3ad 100644
--- a/src/auth.ts
+++ b/src/auth.ts
@@ -10,6 +10,8 @@ export async function verifySession(token: string) {
     if (!token) return null;
+    const decoded = jwt.verify(token, SECRET);
+    return decoded;
-    return false;
 }`,
  },
  {
    name: "JSON Array",
    icon: Layers,
    text: JSON.stringify(
      Array.from({ length: 45 }, (_, i) => ({
        id: i + 1,
        title: `Task item ${i + 1}`,
        status: i === 7 ? "error_failed" : "success",
        timestamp: "2026-10-21T10:00:00Z",
      })),
      null,
      2
    ),
  },
  {
    name: "TypeScript Code",
    icon: FileCode,
    text: `// Service initialization helper
export function initService() {
    const baseUrl = \`https://api.internal/v1\`;
    const wsUrl = \`wss://stream.internal/live\`;
    return { baseUrl, wsUrl };
}

// Service initialization helper
export function initService() {
    const baseUrl = \`https://api.internal/v1\`;
    const wsUrl = \`wss://stream.internal/live\`;
    return { baseUrl, wsUrl };
}`,
  },
];

export default function CompressionPage() {
  const { projects, selectedPath, setSelectedPath } = useProject();

  const [text, setText] = useState("");
  const [compressed, setCompressed] = useState("");
  const [stats, setStats] = useState<CcrStats | null>(null);
  const [list, setList] = useState<CcrEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedRecord, setSelectedRecord] = useState<CcrEntry | null>(null);
  const [copiedKind, setCopiedKind] = useState<string | null>(null);

  const copyToClipboard = (val: string, kind: string) => {
    navigator.clipboard.writeText(val);
    setCopiedKind(kind);
    setTimeout(() => setCopiedKind(null), 2000);
  };

  const refresh = async (r: string) => {
    if (!r) return;
    try {
      const [s, l] = await Promise.all([
        fetch(`/api/compression/stats?root=${encodeURIComponent(r)}`).then((x) => x.json()),
        fetch(`/api/compression?root=${encodeURIComponent(r)}&limit=30`).then((x) => x.json()),
      ]);
      if (!s.error) setStats(s);
      setList(Array.isArray(l) ? l : []);
    } catch (err: any) {
      setError(err.message);
    }
  };

  useEffect(() => {
    if (selectedPath) {
      refresh(selectedPath);
    }
  }, [selectedPath]);

  const handleCompress = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/compress", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text, root: selectedPath }),
      });
      const data = await res.json();
      if (data.error) setError(data.error);
      else {
        setCompressed(data.compressed || "");
        refresh(selectedPath);
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  const pct = (e: CcrEntry) =>
    e.orig_tokens > 0 ? (((e.orig_tokens - e.comp_tokens) / e.orig_tokens) * 100).toFixed(1) + "%" : "0%";

  const getKindBadgeClass = (kind: string) => {
    switch (kind) {
      case "BuildLog":
        return "bg-amber-500/15 text-amber-500 border-amber-500/20";
      case "GitDiff":
        return "bg-emerald-500/15 text-emerald-500 border-emerald-500/20";
      case "JsonArray":
        return "bg-blue-500/15 text-blue-500 border-blue-500/20";
      case "SourceCode":
        return "bg-purple-500/15 text-purple-500 border-purple-500/20";
      default:
        return "bg-muted text-muted-foreground border-border";
    }
  };

  return (
    <div className="space-y-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl flex items-center gap-2">
            <Cpu className="h-6 w-6 text-primary" /> Context Compression & CCR Store
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Lossless Compress-Cache-Retrieve engine for tool outputs, logs, diffs, and large JSON arrays.
          </p>
        </div>

        {/* Project Selector */}
        {projects.length > 0 && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground whitespace-nowrap">Target:</span>
            <select
              aria-label="Target Project"
              value={selectedPath}
              onChange={(e) => setSelectedPath(e.target.value)}
              className="h-9 max-w-[260px] truncate rounded-lg border border-border bg-card px-3 text-xs font-medium text-foreground cursor-pointer focus:ring-1 focus:ring-primary"
            >
              {projects.map((p) => (
                <option key={p.id} value={p.path}>
                  {p.name}
                </option>
              ))}
            </select>
          </div>
        )}
      </div>

      {/* Stats Cards */}
      <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4">
        {[
          { label: "Cached Originals", val: stats?.records.toLocaleString() || "0", sub: "Saved in CCR SQLite" },
          { label: "Original Tokens", val: stats?.original_tokens.toLocaleString() || "0", sub: "Pre-compression" },
          { label: "Compressed Tokens", val: stats?.compressed_tokens.toLocaleString() || "0", sub: "Forwarded to LLM" },
          {
            label: "Tokens Saved",
            val: stats?.saved_tokens.toLocaleString() || "0",
            sub: stats ? `${((1 - stats.compression_ratio) * 100).toFixed(1)}% reduction` : "0%",
            highlight: true,
          },
        ].map((item) => (
          <Card key={item.label} className="border-border/80 shadow-xs glow-card">
            <CardContent className="p-4 sm:p-5">
              <div className="text-xs text-muted-foreground font-medium">{item.label}</div>
              <div
                className={`font-mono text-xl sm:text-2xl font-bold mt-1 ${
                  item.highlight ? "text-emerald-500" : "text-foreground"
                }`}
              >
                {item.val}
              </div>
              <div className="text-[11px] text-muted-foreground mt-0.5">{item.sub}</div>
            </CardContent>
          </Card>
        ))}
      </div>

      {/* Interactive Live Playground */}
      <Card className="border-border/80 shadow-xs">
        <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
          <CardTitle className="text-sm font-semibold flex items-center justify-between">
            <span className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-primary" /> Live Compression Simulator
            </span>
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-[11px] text-muted-foreground hidden sm:inline mr-1">Load sample:</span>
              {PRESETS.map((preset) => {
                const Icon = preset.icon;
                return (
                  <button
                    key={preset.name}
                    type="button"
                    onClick={() => {
                      setText(preset.text);
                      setCompressed("");
                    }}
                    className="inline-flex items-center gap-1 rounded bg-muted px-2 py-1 text-[11px] font-medium text-foreground hover:bg-accent transition-colors cursor-pointer"
                  >
                    <Icon className="h-3 w-3 text-primary" />
                    <span>{preset.name}</span>
                  </button>
                );
              })}
            </div>
          </CardTitle>
        </CardHeader>
        <CardContent className="p-5">
          <form onSubmit={handleCompress} className="space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {/* Input Area */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span className="font-medium">Raw Payload Input</span>
                  <span className="font-mono">{text.length} chars</span>
                </div>
                <Textarea
                  placeholder="Paste build logs, git diffs, JSON arrays, or code here..."
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                  rows={9}
                  className="font-mono text-xs leading-relaxed"
                />
              </div>

              {/* Output Preview */}
              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-xs text-muted-foreground">
                  <span className="font-medium">Compressed Output (Simulated)</span>
                  {compressed && (
                    <span className="font-mono text-emerald-500 font-semibold">
                      {compressed.length} chars ({Math.max(0, Math.round((1 - compressed.length / text.length) * 100))}% saved)
                    </span>
                  )}
                </div>
                <div className="relative rounded-lg border border-border bg-background/80 p-3 h-[216px] overflow-auto scroll-thin font-mono text-xs leading-relaxed text-foreground/90">
                  {compressed ? (
                    <pre className="whitespace-pre-wrap">{compressed}</pre>
                  ) : (
                    <span className="text-muted-foreground/60 italic text-xs">
                      Run compression to preview the result and token reduction...
                    </span>
                  )}
                </div>
              </div>
            </div>

            <div className="flex items-center justify-between pt-2">
              <Button type="submit" disabled={loading || !text.trim()} className="px-5 text-xs font-semibold cursor-pointer">
                {loading ? "Compressing..." : "Compress Payload"}
              </Button>
              {compressed && (
                <button
                  type="button"
                  onClick={() => copyToClipboard(compressed, "live")}
                  className="inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
                >
                  {copiedKind === "live" ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                  <span>Copy compressed output</span>
                </button>
              )}
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Recent CCR Records Table */}
      <Card className="border-border/80 shadow-xs">
        <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
          <CardTitle className="text-sm font-semibold flex items-center justify-between">
            <span className="flex items-center gap-2">
              <Database className="h-4 w-4 text-primary" /> Stored Originals (Retrieve-on-Demand)
            </span>
            <span className="text-xs text-muted-foreground font-mono">
              {list.length} recent entries
            </span>
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0 overflow-x-auto scroll-thin">
          <Table>
            <TableHeader>
              <TableRow className="text-xs">
                <TableHead className="w-32">Handle ID</TableHead>
                <TableHead>Type</TableHead>
                <TableHead className="text-right">Original</TableHead>
                <TableHead className="text-right">Compressed</TableHead>
                <TableHead className="text-right">Reduction</TableHead>
                <TableHead className="text-right">Recorded</TableHead>
                <TableHead className="w-16 text-center">Action</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody className="text-xs">
              {list.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="h-24 text-center text-muted-foreground">
                    No CCR compression records stored yet.
                  </TableCell>
                </TableRow>
              ) : (
                list.map((e) => (
                  <TableRow key={e.id} className="hover:bg-muted/40 transition-colors">
                    <TableCell className="font-mono font-medium text-foreground">
                      #{e.id}
                    </TableCell>
                    <TableCell>
                      <span className={`rounded px-1.5 py-0.5 text-[10px] font-mono border font-medium ${getKindBadgeClass(e.kind)}`}>
                        {e.kind}
                      </span>
                    </TableCell>
                    <TableCell className="text-right font-mono">{e.orig_tokens.toLocaleString()} tok</TableCell>
                    <TableCell className="text-right font-mono">{e.comp_tokens.toLocaleString()} tok</TableCell>
                    <TableCell className="text-right font-mono font-semibold text-emerald-500">
                      {pct(e)}
                    </TableCell>
                    <TableCell className="text-right text-muted-foreground text-[11px]">
                      {e.created_at ? new Date(e.created_at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "—"}
                    </TableCell>
                    <TableCell className="text-center">
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 w-7 p-0 text-muted-foreground hover:text-foreground cursor-pointer"
                        title="View Before/After Comparison"
                        onClick={() => setSelectedRecord(e)}
                      >
                        <Eye className="h-3.5 w-3.5" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Detail Dialog */}
      <Dialog open={!!selectedRecord} onOpenChange={(open) => !open && setSelectedRecord(null)}>
        <DialogContent className="max-w-4xl max-h-[88vh] flex flex-col p-6 overflow-hidden">
          {selectedRecord && (
            <div className="flex flex-col gap-4 overflow-hidden h-full">
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border pb-3 pr-6">
                <div className="flex items-center gap-2">
                  <h3 className="text-base font-bold text-foreground">CCR Compression Record</h3>
                  <code className="rounded bg-muted px-2 py-0.5 font-mono text-xs">
                    #{selectedRecord.id}
                  </code>
                  <span className={`rounded px-1.5 py-0.5 text-[10px] font-mono border font-medium ${getKindBadgeClass(selectedRecord.kind)}`}>
                    {selectedRecord.kind}
                  </span>
                </div>
                {selectedRecord.created_at ? (
                  <span className="text-xs text-muted-foreground font-mono">
                    {new Date(selectedRecord.created_at * 1000).toLocaleString()}
                  </span>
                ) : null}
              </div>

              <div className="grid grid-cols-3 gap-3">
                <div className="p-3 bg-muted/40 border border-border/60 rounded-lg">
                  <div className="text-[11px] text-muted-foreground">Original Size</div>
                  <div className="text-sm font-mono font-bold mt-0.5">{selectedRecord.orig_tokens.toLocaleString()} tokens</div>
                </div>
                <div className="p-3 bg-muted/40 border border-border/60 rounded-lg">
                  <div className="text-[11px] text-muted-foreground">Forwarded to Model</div>
                  <div className="text-sm font-mono font-bold mt-0.5">{selectedRecord.comp_tokens.toLocaleString()} tokens</div>
                </div>
                <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 rounded-lg">
                  <div className="text-[11px] text-emerald-600 dark:text-emerald-400">Tokens Saved</div>
                  <div className="text-sm font-mono font-bold text-emerald-600 dark:text-emerald-400 mt-0.5">
                    {(selectedRecord.orig_tokens - selectedRecord.comp_tokens).toLocaleString()} ({pct(selectedRecord)})
                  </div>
                </div>
              </div>

              <Tabs defaultValue="diff" className="flex-1 flex flex-col overflow-hidden">
                <TabsList className="w-fit">
                  <TabsTrigger value="diff">Side-by-Side View</TabsTrigger>
                  <TabsTrigger value="compressed">Compressed (Sent to Model)</TabsTrigger>
                  <TabsTrigger value="original">Original (Full Content)</TabsTrigger>
                </TabsList>

                <TabsContent value="diff" className="flex-1 overflow-hidden mt-3">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3 h-[45vh]">
                    <div className="flex flex-col border border-border rounded-lg overflow-hidden">
                      <div className="bg-muted/40 px-3 py-1.5 text-[11px] font-medium text-muted-foreground border-b border-border flex justify-between">
                        <span>Original Payload</span>
                        <span>{selectedRecord.original?.length || 0} chars</span>
                      </div>
                      <pre className="flex-1 overflow-auto p-3 text-xs font-mono leading-relaxed bg-background/50 scroll-thin">
                        {selectedRecord.original || "(empty)"}
                      </pre>
                    </div>

                    <div className="flex flex-col border border-border rounded-lg overflow-hidden">
                      <div className="bg-muted/40 px-3 py-1.5 text-[11px] font-medium text-muted-foreground border-b border-border flex justify-between">
                        <span>Compressed Summary</span>
                        <span>{selectedRecord.compressed?.length || 0} chars</span>
                      </div>
                      <pre className="flex-1 overflow-auto p-3 text-xs font-mono leading-relaxed bg-background/50 scroll-thin">
                        {selectedRecord.compressed || "(empty)"}
                      </pre>
                    </div>
                  </div>
                </TabsContent>

                <TabsContent value="compressed" className="flex-1 flex flex-col overflow-hidden mt-3">
                  <div className="flex justify-between items-center mb-1.5 text-xs text-muted-foreground">
                    <span>{selectedRecord.compressed?.length || 0} characters</span>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs gap-1 cursor-pointer"
                      onClick={() => copyToClipboard(selectedRecord.compressed || "", "comp")}
                    >
                      {copiedKind === "comp" ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                      {copiedKind === "comp" ? "Copied" : "Copy"}
                    </Button>
                  </div>
                  <pre className="flex-1 overflow-auto rounded-lg border border-border bg-background/50 p-4 text-xs font-mono leading-relaxed max-h-[48vh] scroll-thin">
                    {selectedRecord.compressed || "(empty)"}
                  </pre>
                </TabsContent>

                <TabsContent value="original" className="flex-1 flex flex-col overflow-hidden mt-3">
                  <div className="flex justify-between items-center mb-1.5 text-xs text-muted-foreground">
                    <span>{selectedRecord.original?.length || 0} characters</span>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs gap-1 cursor-pointer"
                      onClick={() => copyToClipboard(selectedRecord.original || "", "orig")}
                    >
                      {copiedKind === "orig" ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                      {copiedKind === "orig" ? "Copied" : "Copy"}
                    </Button>
                  </div>
                  <pre className="flex-1 overflow-auto rounded-lg border border-border bg-background/50 p-4 text-xs font-mono leading-relaxed max-h-[48vh] scroll-thin">
                    {selectedRecord.original || "(empty)"}
                  </pre>
                </TabsContent>
              </Tabs>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
