"use client";
import { useEffect, useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

interface Project {
  id: string;
  name: string;
  path: string;
  counts: { files: number; symbols: number; edges: number; chunks: number; embeddings: number };
  db_size_mb: number;
}

export default function Home() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [cfg, setCfg] = useState<{
    base_url: string;
    model: string;
    api_key_set: boolean;
    complete: boolean;
    missing: string[];
  } | null>(null);

  useEffect(() => {
    fetch("/api/projects")
      .then((r) => r.json())
      .then((d) => {
        if (Array.isArray(d)) setProjects(d);
        else setError(d.error || "Unexpected response");
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
    fetch("/api/config")
      .then((r) => r.json())
      .then((d) => {
        if (d && !d.error) setCfg(d);
      })
      .catch(() => {});
  }, []);

  const totals = projects.reduce(
    (acc, p) => ({
      files: acc.files + p.counts.files,
      symbols: acc.symbols + p.counts.symbols,
      edges: acc.edges + p.counts.edges,
      chunks: acc.chunks + p.counts.chunks,
      size: acc.size + p.db_size_mb,
    }),
    { files: 0, symbols: 0, edges: 0, chunks: 0, size: 0 }
  );

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex-1">
          <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">imc Dashboard</h1>
          <p className="text-muted-foreground">
            Code intelligence for coding agents: AST index + lexical/vector retrieval + call graph
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Link href="/search" className="inline-flex items-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">
            Search
          </Link>
          <Link href="/compression" className="inline-flex items-center rounded-md border border-input bg-background px-4 py-2 text-sm font-medium hover:bg-secondary">
            Compression
          </Link>
        </div>
      </div>

      {error && (
        <Card>
          <CardContent className="pt-6 text-destructive">{error}</CardContent>
        </Card>
      )}

      {loading ? (
        <p className="text-muted-foreground">Loading projects...</p>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-3 lg:grid-cols-5">
            {[
              ["Projects", projects.length],
              ["Files", totals.files],
              ["Symbols", totals.symbols],
              ["Edges", totals.edges],
              ["Chunks", totals.chunks],
            ].map(([label, val]) => (
              <Card key={String(label)}>
                <CardContent className="pt-6">
                  <div className="text-xs text-muted-foreground">{label}</div>
                  <div className="font-mono text-xl font-bold sm:text-2xl">{Number(val).toLocaleString()}</div>
                </CardContent>
              </Card>
            ))}
          </div>

          {cfg && (
            <Card className={cfg.complete ? "" : "border-amber-500/60"}>
              <CardHeader>
                <CardTitle className="text-base flex items-center justify-between">
                  <span>LLM setup</span>
                  {cfg.complete ? (
                    <Badge variant="secondary">ready</Badge>
                  ) : (
                    <Badge variant="destructive">missing {cfg.missing.join(", ")}</Badge>
                  )}
                </CardTitle>
              </CardHeader>
              <CardContent className="flex flex-wrap items-center gap-x-6 gap-y-1 text-sm">
                <span className="text-muted-foreground">
                  model <span className="font-mono">{cfg.model || "—"}</span>
                </span>
                <span className="text-muted-foreground truncate max-w-[280px]">
                  url <span className="font-mono">{cfg.base_url || "—"}</span>
                </span>
                <span className="text-muted-foreground">
                  key <span className="font-mono">{cfg.api_key_set ? "set" : "—"}</span>
                </span>
                <Link
                  href="/settings"
                  className="ml-auto text-sm text-primary hover:underline"
                >
                  {cfg.complete ? "Change settings" : "Configure now"}
                </Link>
              </CardContent>
            </Card>
          )}

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 sm:gap-6">
            {projects.map((p) => (
              <Card key={p.id}>
                <CardHeader>
                  <CardTitle className="text-lg flex items-center justify-between">
                    <span>{p.name}</span>
                    <Badge variant="secondary">{p.db_size_mb.toFixed(2)} MB</Badge>
                  </CardTitle>
                  <div className="text-xs font-mono text-muted-foreground truncate">{p.path}</div>
                </CardHeader>
                <CardContent className="space-y-2">
                  <div className="grid grid-cols-2 gap-2 text-sm">
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Files</span>
                      <span className="font-mono">{p.counts.files}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Symbols</span>
                      <span className="font-mono">{p.counts.symbols}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Edges</span>
                      <span className="font-mono">{p.counts.edges}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Chunks</span>
                      <span className="font-mono">{p.counts.chunks}</span>
                    </div>
                  </div>
                  <Link
                    href={`/projects/${p.id}`}
                    className="mt-2 block text-center px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
                  >
                    Open Project
                  </Link>
                </CardContent>
              </Card>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
