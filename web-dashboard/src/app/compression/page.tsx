"use client";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

interface Project {
  id: string;
  name: string;
  path: string;
  counts: { files: number; symbols: number; edges: number; chunks: number; embeddings: number };
  db_size_mb: number;
}

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
  orig_tokens: number;
  comp_tokens: number;
  created_at: number;
}

export default function CompressionPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [root, setRoot] = useState("");
  const [text, setText] = useState("");
  const [compressed, setCompressed] = useState("");
  const [stats, setStats] = useState<CcrStats | null>(null);
  const [list, setList] = useState<CcrEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/projects")
      .then((r) => r.json())
      .then((d) => {
        if (Array.isArray(d)) {
          setProjects(d);
          if (d.length > 0) setRoot(d[0].path);
        }
      })
      .catch(() => {});
  }, []);

  const refresh = async (r: string) => {
    if (!r) return;
    try {
      const [s, l] = await Promise.all([
        fetch(`/api/compression/stats?root=${encodeURIComponent(r)}`).then((x) => x.json()),
        fetch(`/api/compression?root=${encodeURIComponent(r)}&limit=20`).then((x) => x.json()),
      ]);
      if (!s.error) setStats(s);
      setList(Array.isArray(l) ? l : []);
    } catch (err: any) {
      setError(err.message);
    }
  };

  useEffect(() => {
    refresh(root);
  }, [root]);

  const handleCompress = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/compress", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text, root }),
      });
      const data = await res.json();
      if (data.error) setError(data.error);
      else setCompressed(data.compressed || "");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  const pct = (e: CcrEntry) =>
    e.orig_tokens > 0 ? (((e.orig_tokens - e.comp_tokens) / e.orig_tokens) * 100).toFixed(1) + "%" : "0%";

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Compression</h1>

      <div className="w-full max-w-2xl">
        <label className="text-xs text-muted-foreground">Project (CCR store)</label>
        <Select value={root} onValueChange={setRoot}>
          <SelectTrigger className="w-full">
            <SelectValue placeholder="Select project" />
          </SelectTrigger>
          <SelectContent>
            {projects.map((p) => (
              <SelectItem key={p.id} value={p.path}>{p.name} — {p.path}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {error && <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>}

      <Card>
        <CardHeader><CardTitle>CCR Totals</CardTitle></CardHeader>
        <CardContent>
          {stats ? (
            <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4">
              {[
                ["Records", stats.records.toLocaleString()],
                ["Original tokens", stats.original_tokens.toLocaleString()],
                ["Compressed tokens", stats.compressed_tokens.toLocaleString()],
                ["Saved tokens", stats.saved_tokens.toLocaleString()],
              ].map(([k, v]) => (
                <div key={k} className="p-3 bg-muted rounded-lg">
                  <div className="text-xs text-muted-foreground">{k}</div>
                  <div className="text-lg font-mono">{v}</div>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-muted-foreground">No CCR records for this project.</p>
          )}
          {stats && (
            <p className="text-xs text-muted-foreground mt-3">
              avg ratio {stats.compression_ratio.toFixed(3)} · db {(stats.db_bytes / 1024).toFixed(1)} KB · {stats.db}
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>Compress Text</CardTitle></CardHeader>
        <CardContent>
          <form onSubmit={handleCompress} className="flex w-full max-w-2xl flex-col gap-4">
            <Textarea
              placeholder="Paste build log, JSON array, source code, or git diff..."
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={6}
            />
            <Button type="submit" disabled={loading}>
              {loading ? "Compressing..." : "Compress"}
            </Button>
            {compressed && (
              <div>
                <div className="text-xs text-muted-foreground mb-1">
                  Output ({compressed.length} chars, was {text.length})
                </div>
                <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 text-[11px] font-mono sm:p-3 sm:text-xs">{compressed}</pre>
              </div>
            )}
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>Recent Records</CardTitle></CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="whitespace-nowrap">ID</TableHead>
                <TableHead className="whitespace-nowrap">Kind</TableHead>
                <TableHead className="text-right">Orig</TableHead>
                <TableHead className="text-right">Comp</TableHead>
                <TableHead className="text-right">Saved</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {list.map((e) => (
                <TableRow key={e.id}>
                  <TableCell className="font-mono text-xs">{e.id}</TableCell>
                  <TableCell><Badge variant="secondary">{e.kind}</Badge></TableCell>
                  <TableCell className="text-right font-mono text-sm">{e.orig_tokens}</TableCell>
                  <TableCell className="text-right font-mono text-sm">{e.comp_tokens}</TableCell>
                  <TableCell className="text-right font-mono text-sm">{pct(e)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
