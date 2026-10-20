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
      <h1 className="text-3xl font-bold tracking-tight">Search</h1>

      <div className="max-w-2xl">
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

      <form onSubmit={handleSearch} className="flex gap-2 max-w-2xl">
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
                    <div className="font-mono text-sm break-all">{h.file}:{h.start_line}-{h.end_line}</div>
                    <Badge variant="secondary">{h.source}</Badge>
                  </div>
                  <div className="flex items-center gap-2 text-xs text-muted-foreground mt-1">
                    {h.sym_name && <span className="font-mono">{h.sym_name}</span>}
                    {h.sym_kind && <Badge variant="outline" className="text-[10px]">{h.sym_kind}</Badge>}
                    <span>score {h.score.toFixed(4)}</span>
                  </div>
                  <pre className="text-xs mt-2 bg-muted p-3 rounded overflow-auto max-h-72 whitespace-pre-wrap">{h.text}</pre>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
