"use client";
import { useEffect, useState } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

interface Project {
  id: string;
  name: string;
  path: string;
  counts: { files: number; symbols: number; edges: number; chunks: number; embeddings: number };
  db_size_mb: number;
}

interface SearchHit {
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

interface MemoryEntry {
  id: number;
  content: string;
  mem_type: string;
  category: string;
  importance: number;
  source: string;
  use_count: number;
}

interface TraceNode {
  id: string;
  name: string;
  file: string;
  line: number;
  type: string;
  callees?: TraceNode[];
  callers?: TraceNode[];
  depth: number;
}

interface TraceArm {
  mode: string;
  tree: TraceNode;
}

export default function ProjectDetail({ id }: { id: string }) {
  const [project, setProject] = useState<Project | null>(null);
  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [traceTarget, setTraceTarget] = useState("");
  const [traceArms, setTraceArms] = useState<TraceArm[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/projects")
      .then((r) => r.json())
      .then((list: Project[]) => {
        const p = Array.isArray(list) ? list.find((x) => x.id === id || x.name === id) : null;
        setProject(p || null);
        if (p) {
          fetch(`/api/memory?root=${encodeURIComponent(p.path)}`)
            .then((r) => r.json())
            .then((m) => setMemories(Array.isArray(m) ? m : []))
            .catch(() => {});
        }
      })
      .catch((e) => setError(e.message));
  }, [id]);

  const doSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim() || !project) return;
    setBusy(true);
    setError(null);
    try {
      const r = await fetch("/api/search", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query, root: project.path, limit: 10 }),
      });
      const d = await r.json();
      if (Array.isArray(d)) setHits(d);
      else setError(d.error || "Search failed");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setBusy(false);
    }
  };

  const doTrace = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!traceTarget.trim() || !project) return;
    setBusy(true);
    setError(null);
    try {
      const r = await fetch("/api/trace", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ target: traceTarget, root: project.path, callers: true }),
      });
      const d = await r.json();
      if (d.results) setTraceArms(d.results);
      else setError(d.error || "Trace failed");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setBusy(false);
    }
  };

  const renderTree = (node: TraceNode, depth = 0): React.ReactNode => {
    const kids = [...(node.callees || []), ...(node.callers || [])];
    return (
      <div key={`${node.id}-${depth}`} style={{ marginLeft: depth * 16 }}>
        <div className="flex items-center gap-2 py-0.5">
          <Badge variant="secondary" className="text-[10px]">{node.type}</Badge>
          <span className="font-mono text-sm">{node.name}</span>
          <span className="text-xs text-muted-foreground">
            {node.file}:{node.line}
          </span>
        </div>
        {kids.map((k) => renderTree(k, depth + 1))}
      </div>
    );
  };

  if (error && !project) {
    return (
      <div className="space-y-4">
        <Link href="/projects" className="text-sm text-primary hover:underline">← Back</Link>
        <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>
      </div>
    );
  }

  if (!project) return <p className="text-muted-foreground">Loading...</p>;

  const c = project.counts;
  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <Link href="/projects" className="text-sm text-primary hover:underline">← Projects</Link>
          <h1 className="text-3xl font-bold tracking-tight mt-2">{project.name}</h1>
          <p className="text-xs font-mono text-muted-foreground">{project.path}</p>
        </div>
        <Badge variant="secondary">{project.db_size_mb.toFixed(2)} MB</Badge>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        {[
          ["Files", c.files],
          ["Symbols", c.symbols],
          ["Edges", c.edges],
          ["Chunks", c.chunks],
          ["Embeddings", c.embeddings],
        ].map(([label, val]) => (
          <Card key={String(label)}>
            <CardContent className="pt-6">
              <div className="text-xs text-muted-foreground">{label}</div>
              <div className="text-2xl font-bold font-mono">{Number(val).toLocaleString()}</div>
            </CardContent>
          </Card>
        ))}
      </div>

      {error && <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>}

      <Tabs defaultValue="search">
        <TabsList>
          <TabsTrigger value="search">Search</TabsTrigger>
          <TabsTrigger value="trace">Trace</TabsTrigger>
          <TabsTrigger value="memory">Memory ({memories.length})</TabsTrigger>
        </TabsList>

        <TabsContent value="search" className="space-y-4">
          <form onSubmit={doSearch} className="flex gap-2 max-w-2xl">
            <Input
              placeholder="e.g. authentication flow"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            <Button type="submit" disabled={busy}>Search</Button>
          </form>
          {hits.length > 0 && (
            <Card>
              <CardHeader><CardTitle>Results ({hits.length})</CardTitle></CardHeader>
              <CardContent className="space-y-4">
                {hits.map((h, i) => (
                  <div key={i} className="border-b pb-4 last:border-0">
                    <div className="flex justify-between items-start gap-2">
                      <div className="font-mono text-sm break-all">{h.file}:{h.start_line}-{h.end_line}</div>
                      <Badge variant="secondary">{h.source}</Badge>
                    </div>
                    <div className="flex items-center gap-2 text-xs text-muted-foreground mt-1">
                      {h.sym_name && <span className="font-mono">{h.sym_name}</span>}
                      {h.sym_kind && <Badge variant="outline" className="text-[10px]">{h.sym_kind}</Badge>}
                      <span>score {h.score.toFixed(4)}</span>
                    </div>
                    <pre className="text-xs mt-2 bg-muted p-3 rounded overflow-auto max-h-64 whitespace-pre-wrap">{h.text}</pre>
                  </div>
                ))}
              </CardContent>
            </Card>
          )}
        </TabsContent>

        <TabsContent value="trace" className="space-y-4">
          <form onSubmit={doTrace} className="flex gap-2 max-w-2xl">
            <Input
              placeholder="Symbol name (callers reverse-trace)"
              value={traceTarget}
              onChange={(e) => setTraceTarget(e.target.value)}
            />
            <Button type="submit" disabled={busy}>Trace</Button>
          </form>
          {traceArms.map((arm, i) => (
            <Card key={i}>
              <CardHeader><CardTitle className="text-base">{arm.mode}</CardTitle></CardHeader>
              <CardContent>{renderTree(arm.tree)}</CardContent>
            </Card>
          ))}
        </TabsContent>

        <TabsContent value="memory">
          <Card>
            <CardHeader><CardTitle>Memories</CardTitle></CardHeader>
            <CardContent>
              {memories.length === 0 ? (
                <p className="text-muted-foreground">No memories yet.</p>
              ) : (
                <div className="space-y-3">
                  {memories.map((m) => (
                    <div key={m.id} className="border-b pb-3 last:border-0">
                      <div className="flex gap-2 items-center flex-wrap">
                        <Badge variant="secondary">{m.mem_type}</Badge>
                        <Badge variant="outline" className="text-[10px]">{m.category}</Badge>
                        <span className="text-xs text-muted-foreground">source={m.source} · used {m.use_count}× · imp {m.importance}</span>
                      </div>
                      <div className="text-sm mt-1">{m.content}</div>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
