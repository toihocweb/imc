"use client";

import { useEffect, useState, Suspense } from "react";
import { useSearchParams, useRouter } from "next/navigation";
import Link from "next/link";
import {
  Search as SearchIcon,
  FileCode,
  Code2,
  Copy,
  Check,
  Network,
  Sparkles,
  Layers,
  ArrowRight,
  Filter,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { useProject } from "@/components/project-context";

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

function SearchContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { projects, selectedPath, setSelectedPath } = useProject();

  const [query, setQuery] = useState(searchParams.get("q") || "");
  const [limit, setLimit] = useState(10);
  const [filterType, setFilterType] = useState<"all" | "function" | "class" | "doc">("all");
  const [results, setResults] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

  const executeSearch = async (targetQuery: string, targetPath: string, searchLimit: number) => {
    if (!targetQuery.trim() || !targetPath) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/search", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query: targetQuery, root: targetPath, limit: searchLimit }),
      });
      const data = await res.json();
      if (Array.isArray(data)) {
        setResults(data);
      } else {
        setError(data.error || "Search returned an unexpected response");
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    const q = searchParams.get("q");
    if (q && selectedPath) {
      setQuery(q);
      executeSearch(q, selectedPath, limit);
    }
  }, [searchParams, selectedPath]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!query.trim() || !selectedPath) return;
    executeSearch(query, selectedPath, limit);
  };

  const copyCitation = (file: string, start: number, end: number, idx: number) => {
    const citation = `${file}:${start}-${end}`;
    navigator.clipboard.writeText(citation);
    setCopiedIndex(idx);
    setTimeout(() => setCopiedIndex(null), 2000);
  };

  const filteredResults = results.filter((hit) => {
    if (filterType === "all") return true;
    if (filterType === "function") {
      const k = hit.sym_kind?.toLowerCase() || "";
      return k.includes("function") || k.includes("method");
    }
    if (filterType === "class") {
      const k = hit.sym_kind?.toLowerCase() || "";
      return k.includes("class") || k.includes("struct") || k.includes("interface");
    }
    if (filterType === "doc") {
      return hit.file.endsWith(".md") || hit.file.endsWith(".txt");
    }
    return true;
  });

  return (
    <div className="space-y-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl flex items-center gap-2">
            <SearchIcon className="h-6 w-6 text-primary" /> Codebase Search
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Hybrid lexical BM25 + vector search with AST evidence and symbol ranking.
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

      {/* Search Input Box */}
      <Card className="border-border/80 shadow-xs">
        <CardContent className="p-4 sm:p-5">
          <form onSubmit={handleSubmit} className="space-y-3">
            <div className="relative flex items-center">
              <SearchIcon className="absolute left-3.5 h-4 w-4 text-muted-foreground" />
              <input
                type="text"
                placeholder="Search by natural query or exact symbol (e.g. 'authentication flow', 'verify_token')..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="h-11 w-full rounded-lg border border-border/90 bg-background pl-10 pr-24 text-sm text-foreground transition-all hover:border-primary/50 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/20"
              />
              <Button
                type="submit"
                disabled={loading || !selectedPath}
                className="absolute right-1.5 h-8 px-4 text-xs font-semibold cursor-pointer"
              >
                {loading ? "Searching..." : "Search"}
              </Button>
            </div>

            <div className="flex flex-wrap items-center justify-between gap-2 pt-1 text-xs text-muted-foreground">
              <div className="flex items-center gap-2">
                <span>Result limit:</span>
                {[10, 20, 30].map((n) => (
                  <button
                    key={n}
                    type="button"
                    onClick={() => setLimit(n)}
                    className={`rounded px-2 py-0.5 text-xs font-medium transition-colors ${
                      limit === n
                        ? "bg-primary text-primary-foreground font-semibold"
                        : "bg-muted hover:bg-accent text-muted-foreground"
                    }`}
                  >
                    {n}
                  </button>
                ))}
              </div>

              <div className="flex items-center gap-1.5 text-[11px]">
                <Sparkles className="h-3 w-3 text-primary" />
                <span>BM25 + Semantic Cosine Fusion</span>
              </div>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Error state */}
      {error && (
        <Card className="border-destructive/40 bg-destructive/10 text-destructive">
          <CardContent className="p-4 text-xs font-medium">{error}</CardContent>
        </Card>
      )}

      {/* Filter Tabs & Results Count */}
      {results.length > 0 && (
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pt-2">
          <div className="flex items-center gap-1.5 rounded-lg border border-border bg-card p-1 text-xs">
            <Filter className="h-3.5 w-3.5 text-muted-foreground ml-1.5 mr-0.5" />
            {(
              [
                ["all", "All Matches"],
                ["function", "Functions / Methods"],
                ["class", "Classes / Types"],
                ["doc", "Documentation"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setFilterType(key)}
                className={`rounded-md px-2.5 py-1 font-medium transition-colors cursor-pointer ${
                  filterType === key
                    ? "bg-primary/15 text-primary font-semibold"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground"
                }`}
              >
                {label}
              </button>
            ))}
          </div>

          <div className="text-xs text-muted-foreground font-mono">
            Showing {filteredResults.length} of {results.length} hits
          </div>
        </div>
      )}

      {/* Results List */}
      {results.length > 0 && (
        <div className="space-y-4">
          {filteredResults.map((hit, idx) => {
            const hasGraphEvidence = hit.text.includes("// graph:");
            const mainText = hasGraphEvidence
              ? hit.text.split("\n// graph:")[0]
              : hit.text;
            const graphText = hasGraphEvidence
              ? hit.text.split("\n// graph:")[1]
              : null;

            return (
              <div
                key={`${hit.file}-${hit.start_line}-${idx}`}
                className="glow-card rounded-xl border border-border/80 bg-card overflow-hidden shadow-xs"
              >
                {/* Header bar */}
                <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/20 px-4 py-2.5 text-xs">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="flex h-5 w-5 items-center justify-center rounded bg-primary/10 text-primary font-mono text-[10px] font-bold">
                      #{idx + 1}
                    </span>
                    <FileCode className="h-4 w-4 text-muted-foreground shrink-0" />
                    <span className="font-mono font-medium text-foreground truncate">
                      {hit.file}
                    </span>
                    <span className="font-mono text-muted-foreground shrink-0">
                      :{hit.start_line}-{hit.end_line}
                    </span>
                  </div>

                  <div className="flex items-center gap-2">
                    {hit.sym_name && (
                      <Badge variant="outline" className="font-mono text-[11px] gap-1 bg-background/80">
                        <Code2 className="h-3 w-3 text-primary" />
                        {hit.sym_name}
                      </Badge>
                    )}
                    {hit.sym_kind && (
                      <Badge variant="secondary" className="text-[10px] font-medium">
                        {hit.sym_kind}
                      </Badge>
                    )}
                    <span className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                      score {hit.score.toFixed(3)}
                    </span>
                  </div>
                </div>

                {/* Code body */}
                <div className="relative">
                  <pre className="max-h-80 overflow-auto bg-background/60 p-4 font-mono text-xs leading-relaxed text-foreground/90 scroll-thin select-text">
                    {mainText}
                  </pre>
                </div>

                {/* Graph evidence & actions footer */}
                <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 bg-muted/10 px-4 py-2 text-xs">
                  {graphText ? (
                    <div className="flex items-center gap-1.5 text-[11px] text-primary truncate max-w-xl">
                      <Network className="h-3 w-3 shrink-0" />
                      <span className="truncate">graph: {graphText.trim()}</span>
                    </div>
                  ) : (
                    <span className="text-[11px] text-muted-foreground">
                      Matched via {hit.source}
                    </span>
                  )}

                  <div className="flex items-center gap-2 ml-auto">
                    {hit.sym_name && (
                      <Link
                        href={`/trace?target=${encodeURIComponent(hit.sym_name)}`}
                        className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline"
                      >
                        <Network className="h-3 w-3" /> Trace Callers
                      </Link>
                    )}
                    <button
                      type="button"
                      onClick={() => copyCitation(hit.file, hit.start_line, hit.end_line, idx)}
                      className="inline-flex items-center gap-1 rounded border border-border px-2 py-1 text-[11px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground transition-colors cursor-pointer"
                    >
                      {copiedIndex === idx ? (
                        <>
                          <Check className="h-3 w-3 text-emerald-500" />
                          <span className="text-emerald-500">Copied</span>
                        </>
                      ) : (
                        <>
                          <Copy className="h-3 w-3" />
                          <span>Copy Citation</span>
                        </>
                      )}
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Empty result hint */}
      {!loading && results.length === 0 && query && (
        <Card className="p-8 text-center border-dashed">
          <p className="text-sm font-medium text-foreground">No matching code chunks found</p>
          <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
            Try adjusting keywords or search for broader concepts.
          </p>
        </Card>
      )}
    </div>
  );
}

export default function SearchPage() {
  return (
    <Suspense fallback={<div className="text-xs text-muted-foreground">Loading search...</div>}>
      <SearchContent />
    </Suspense>
  );
}
