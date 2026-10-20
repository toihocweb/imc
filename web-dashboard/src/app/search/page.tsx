"use client";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

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

export default function SearchPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [root, setRoot] = useState<string>("");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchHit[]>([]);
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

  const handleSearch = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim() || !root) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/search", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query, root, limit: 10 }),
      });
      const data = await res.json();
      if (Array.isArray(data)) setResults(data);
      else setError(data.error || "Search failed");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Search</h1>

      <div className="w-full max-w-2xl">
        <label className="text-xs text-muted-foreground">Project</label>
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

      <form onSubmit={handleSearch} className="flex w-full max-w-2xl flex-col gap-2 sm:flex-row">
        <Input
          placeholder="Search query (e.g. authentication flow)"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Button type="submit" disabled={loading || !root}>
          {loading ? "Searching..." : "Search"}
        </Button>
      </form>

      {error && <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>}

      {results.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Results ({results.length})</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-4">
              {results.map((h, i) => (
                <div key={i} className="border-b pb-4 last:border-0">
                  <div className="flex justify-between items-start gap-2">
                    <div className="break-all font-mono text-xs sm:text-sm">{h.file}:{h.start_line}-{h.end_line}</div>
                    <Badge variant="secondary">{h.source}</Badge>
                  </div>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground mt-1">
                    {h.sym_name && <span className="font-mono">{h.sym_name}</span>}
                    {h.sym_kind && <Badge variant="outline" className="text-[10px]">{h.sym_kind}</Badge>}
                    <span>score {h.score.toFixed(4)}</span>
                  </div>
                  <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 text-[11px] leading-relaxed sm:p-3 sm:text-xs">{h.text}</pre>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
