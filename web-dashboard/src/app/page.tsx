"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  Search,
  Network,
  Cpu,
  Brain,
  FolderGit2,
  FileCode,
  Code2,
  GitFork,
  Layers,
  ArrowRight,
  Terminal,
  Activity,
  CheckCircle2,
  AlertTriangle,
  ExternalLink,
  Sparkles,
  BarChart,
  Trash2,
  RotateCw,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useProject } from "@/components/project-context";
import { formatNumber, formatTimestamp, formatPercent } from "@/lib/format";
import { UsageData } from "@/lib/imc";

export default function Home() {
  const router = useRouter();
  const { projects, loading, selectedPath, setSelectedPath } = useProject();
  const [searchQuery, setSearchQuery] = useState("");
  const [cfg, setCfg] = useState<{
    base_url: string;
    model: string;
    api_key_set: boolean;
    complete: boolean;
    missing: string[];
  } | null>(null);
  const [usage, setUsage] = useState<UsageData | null>(null);
  const [selectedModel, setSelectedModel] = useState<string>("all");
  const [usagePage, setUsagePage] = useState(1);
  const [usagePageSize, setUsagePageSize] = useState(10);
  const [refreshingUsage, setRefreshingUsage] = useState(false);

  const refreshUsage = async () => {
    setRefreshingUsage(true);
    try {
      const res = await fetch("/api/usage");
      const data = await res.json();
      if (data && !data.error) setUsage(data);
    } catch {} finally {
      setRefreshingUsage(false);
    }
  };

  const handleClearUsage = async () => {
    try {
      await fetch("/api/usage", { method: "DELETE" });
      setSelectedModel("all");
      setUsagePage(1);
      refreshUsage();
    } catch {}
  };

  useEffect(() => {
    fetch("/api/config")
      .then((r) => r.json())
      .then((d) => {
        if (d && !d.error) setCfg(d);
      })
      .catch(() => {});
    refreshUsage();
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

  const handleHeroSearch = (e: React.FormEvent) => {
    e.preventDefault();
    if (!searchQuery.trim()) return;
    router.push(`/search?q=${encodeURIComponent(searchQuery.trim())}`);
  };

  return (
    <div className="w-full space-y-6 animate-in fade-in duration-200">
      {/* Hero Section with Quick Search */}
      <div className="relative overflow-hidden rounded-2xl border border-border/80 bg-gradient-to-b from-card to-card/60 p-6 sm:p-8 lg:p-10 shadow-xs">
        <div className="absolute top-0 right-0 -mt-10 -mr-10 h-72 w-72 rounded-full bg-primary/10 blur-3xl pointer-events-none" />
        <div className="relative z-10 max-w-3xl space-y-4">
          <div className="inline-flex items-center gap-2 rounded-full border border-primary/25 bg-primary/10 px-3 py-1 text-xs font-semibold text-primary">
            <Sparkles className="h-3.5 w-3.5" />
            <span>Code Intelligence for AI Coding Agents</span>
          </div>

          <h1 className="text-3xl font-extrabold tracking-tight sm:text-4xl lg:text-5xl text-foreground">
            Explore, trace, and compress your codebase.
          </h1>

          <p className="text-sm sm:text-base text-muted-foreground leading-relaxed max-w-2xl">
            Tree-sitter AST index, lexical BM25 + vector hybrid retrieval, call-graph blast radius analysis, and proxy context optimization — all in one local SQLite database.
          </p>

          {/* Integrated Search Input */}
          <form onSubmit={handleHeroSearch} className="pt-2">
            <div className="relative flex max-w-2xl items-center">
              <Search className="absolute left-3.5 h-4 w-4 text-muted-foreground" />
              <input
                type="text"
                placeholder="Search repository (e.g. authentication flow, parse token, session store)..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="h-12 w-full rounded-xl border border-border/90 bg-background/80 pl-10 pr-28 text-sm text-foreground shadow-inner transition-all hover:border-primary/50 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/20 backdrop-blur-xs"
              />
              <button
                type="submit"
                className="absolute right-1.5 h-9 rounded-lg bg-primary px-4 text-xs font-medium text-primary-foreground shadow-xs hover:bg-primary/90 transition-colors cursor-pointer"
              >
                Search
              </button>
            </div>
            <div className="flex flex-wrap items-center gap-2 pt-2.5 text-xs text-muted-foreground">
              <span>Quick suggestions:</span>
              {["auth token", "database migration", "route handler", "error retry"].map((sample) => (
                <button
                  key={sample}
                  type="button"
                  onClick={() => router.push(`/search?q=${encodeURIComponent(sample)}`)}
                  className="rounded-md border border-border/60 bg-muted/50 px-2 py-0.5 hover:bg-accent hover:text-foreground transition-colors cursor-pointer"
                >
                  {sample}
                </button>
              ))}
            </div>
          </form>
        </div>
      </div>

      {/* KPI Metrics Grid */}
      <div className="grid grid-cols-2 gap-3 sm:gap-4 md:grid-cols-3 lg:grid-cols-5">
        {[
          { label: "Indexed Projects", val: projects.length, icon: FolderGit2, sub: "Local repos" },
          { label: "Total Files", val: totals.files, icon: FileCode, sub: "Source files" },
          { label: "AST Symbols", val: totals.symbols, icon: Code2, sub: "Functions & types" },
          { label: "Call Graph Edges", val: totals.edges, icon: GitFork, sub: "Caller/callee links" },
          { label: "Search Chunks", val: totals.chunks, icon: Layers, sub: `${totals.size.toFixed(1)} MB index` },
        ].map((item) => {
          const Icon = item.icon;
          return (
            <Card key={item.label} className="glow-card border-border/80">
              <CardContent className="p-4 sm:p-5 flex flex-col justify-between h-full">
                <div className="flex items-center justify-between text-muted-foreground">
                  <span className="text-xs font-medium">{item.label}</span>
                  <div className="p-1.5 rounded-md bg-muted/60">
                    <Icon className="h-4 w-4" />
                  </div>
                </div>
                <div className="pt-2">
                  <div className="font-mono text-2xl font-bold tracking-tight text-foreground">
                    {loading ? "..." : item.val.toLocaleString()}
                  </div>
                  <div className="text-[11px] text-muted-foreground">{item.sub}</div>
                </div>
              </CardContent>
            </Card>
          );
        })}
      </div>

      {/* Live System Engine & Quick Actions */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        {/* Engine Status */}
        <Card className="border-border/80 lg:col-span-1 flex flex-col justify-between">
          <CardHeader className="pb-3">
            <CardTitle className="text-sm font-semibold flex items-center justify-between">
              <span className="flex items-center gap-2">
                <Activity className="h-4 w-4 text-primary" />
                Runtime Engine Status
              </span>
              <span className="flex items-center gap-1.5 text-xs text-emerald-500 font-medium">
                <span className="h-2 w-2 rounded-full bg-emerald-500 animate-pulse" />
                Active
              </span>
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3.5 text-xs text-muted-foreground">
            <div className="flex items-center justify-between border-b border-border/50 pb-2">
              <span>Reverse Proxy</span>
              <span className="font-mono text-foreground font-medium">http://127.0.0.1:35711</span>
            </div>
            <div className="flex items-center justify-between border-b border-border/50 pb-2">
              <span>Upstream Engine</span>
              <span className="font-mono text-foreground font-medium truncate max-w-[160px]">
                Hyperspace (6655)
              </span>
            </div>
            <div className="flex items-center justify-between border-b border-border/50 pb-2">
              <span>API Key Status</span>
              {cfg?.api_key_set ? (
                <span className="flex items-center gap-1 text-emerald-500 font-medium">
                  <CheckCircle2 className="h-3 w-3" /> Configured
                </span>
              ) : (
                <span className="flex items-center gap-1 text-amber-500 font-medium">
                  <AlertTriangle className="h-3 w-3" /> Needs Secret
                </span>
              )}
            </div>
            <div className="pt-1">
              <Link
                href="/settings"
                className="inline-flex w-full items-center justify-center gap-1 rounded-md border border-border bg-muted/40 py-1.5 text-xs font-medium text-foreground hover:bg-accent transition-colors"
              >
                Configure Settings & Models <ArrowRight className="h-3 w-3 ml-1" />
              </Link>
            </div>
          </CardContent>
        </Card>

        {/* Feature Navigation Cards */}
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 lg:col-span-2">
          {[
            {
              title: "Code Retrieval",
              desc: "Hybrid BM25 + Vector search with symbol rank boosting and AST evidence.",
              href: "/search",
              icon: Search,
              action: "Open Search",
            },
            {
              title: "Call Graph & Blast Radius",
              desc: "Trace callees (what it calls) or callers (what breaks when you edit).",
              href: "/trace",
              icon: Network,
              action: "Explore Call Tree",
            },
            {
              title: "Context Compression",
              desc: "CCR store and live text/log/diff compressor saving up to 80% context tokens.",
              href: "/compression",
              icon: Cpu,
              action: "View Compression",
            },
            {
              title: "Durable Project Memory",
              desc: "Auto-learned lessons, mistakes, and architectural decisions kept in SQLite.",
              href: "/memory",
              icon: Brain,
              action: "Inspect Memories",
            },
          ].map((feature) => {
            const Icon = feature.icon;
            return (
              <Link
                key={feature.title}
                href={feature.href}
                className="glow-card group flex flex-col justify-between rounded-xl border border-border/80 bg-card p-4 transition-all hover:bg-card/80"
              >
                <div>
                  <div className="flex items-center justify-between mb-2">
                    <span className="font-semibold text-sm text-foreground group-hover:text-primary transition-colors flex items-center gap-2">
                      <Icon className="h-4 w-4 text-primary" />
                      {feature.title}
                    </span>
                    <ArrowRight className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-1 group-hover:text-primary" />
                  </div>
                  <p className="text-xs text-muted-foreground leading-normal">{feature.desc}</p>
                </div>
                <div className="pt-3 text-[11px] font-medium text-primary">
                  {feature.action} &rarr;
                </div>
              </Link>
            );
          })}
        </div>
      </div>

      {/* Token Usage & Traffic Analysis */}
      <div className="space-y-4">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div>
            <h2 className="text-lg font-bold tracking-tight text-foreground flex items-center gap-2">
              <BarChart className="h-5 w-5 text-primary" /> Token Usage & Traffic
            </h2>
            <p className="text-xs text-muted-foreground">
              Total prompt tokens, generation tokens, and prompt cache hit rate tracked in IMC root.
            </p>
          </div>
          <div className="flex items-center gap-2 self-start sm:self-auto">
            <Button
              variant="outline"
              size="sm"
              onClick={refreshUsage}
              disabled={refreshingUsage}
              className="h-8 px-2.5 text-xs text-muted-foreground hover:text-foreground cursor-pointer"
              title="Refresh token usage"
            >
              <RotateCw className={`h-3.5 w-3.5 mr-1.5 ${refreshingUsage ? "animate-spin" : ""}`} />
              Refresh
            </Button>
            {usage && usage.total_requests > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleClearUsage}
                className="h-8 px-3 text-xs text-muted-foreground hover:text-destructive hover:bg-destructive/10 cursor-pointer"
              >
                <Trash2 className="h-3.5 w-3.5 mr-1" /> Clear Usage Log
              </Button>
            )}
          </div>
        </div>

        {/* Usage KPI Grid */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <Card className="border-border/80 glow-card">
            <CardContent className="p-4 flex flex-col justify-between">
              <span className="text-xs text-muted-foreground font-medium">Total Requests</span>
              <div className="pt-2">
                <div className="font-mono text-xl sm:text-2xl font-bold text-foreground">
                  {formatNumber(usage?.total_requests ?? 0)}
                </div>
                <div className="text-[11px] text-muted-foreground">LLM interactions</div>
              </div>
            </CardContent>
          </Card>

          <Card className="border-border/80 glow-card">
            <CardContent className="p-4 flex flex-col justify-between">
              <span className="text-xs text-muted-foreground font-medium">Input Tokens</span>
              <div className="pt-2">
                <div className="font-mono text-xl sm:text-2xl font-bold text-foreground">
                  {formatNumber(usage?.total_input_tokens ?? 0)}
                </div>
                <div className="text-[11px] text-muted-foreground">Uncached prompt tokens</div>
              </div>
            </CardContent>
          </Card>

          <Card className="border-border/80 glow-card">
            <CardContent className="p-4 flex flex-col justify-between">
              <span className="text-xs text-muted-foreground font-medium">Output Tokens</span>
              <div className="pt-2">
                <div className="font-mono text-xl sm:text-2xl font-bold text-foreground">
                  {formatNumber(usage?.total_output_tokens ?? 0)}
                </div>
                <div className="text-[11px] text-muted-foreground">Generated tokens</div>
              </div>
            </CardContent>
          </Card>

          <Card className="border-border/80 glow-card">
            <CardContent className="p-4 flex flex-col justify-between">
              <div className="flex items-center justify-between">
                <span className="text-xs text-muted-foreground font-medium">Prompt Cache</span>
                <span className="text-[10px] font-semibold text-emerald-500 bg-emerald-500/10 px-1.5 py-0.5 rounded">
                  {formatPercent(usage?.cache_hit_rate ?? 0)} hit
                </span>
              </div>
              <div className="pt-2">
                <div className="font-mono text-xl sm:text-2xl font-bold text-emerald-500">
                  {formatNumber(usage?.total_cache_read_tokens ?? 0)}
                </div>
                <div className="text-[11px] text-muted-foreground">Tokens served from cache (0.1×)</div>
              </div>
            </CardContent>
          </Card>
        </div>

        {/* Model Breakdown & Recent Requests */}
        {usage && usage.total_requests > 0 ? (() => {
          const filteredRecords =
            selectedModel === "all"
              ? usage.records
              : usage.records.filter((r) => r.model === selectedModel);
          const activeModelStats =
            selectedModel !== "all" ? usage.by_model[selectedModel] : null;
          const totalPages = Math.max(1, Math.ceil(filteredRecords.length / usagePageSize));

          return (
            <Card className="border-border/80 shadow-xs overflow-hidden">
              <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <CardTitle className="text-sm font-semibold">Recent Requests & Breakdown</CardTitle>
                    <button
                      type="button"
                      onClick={refreshUsage}
                      disabled={refreshingUsage}
                      className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors cursor-pointer"
                      title="Refresh records"
                    >
                      <RotateCw className={`h-3.5 w-3.5 ${refreshingUsage ? "animate-spin" : ""}`} />
                    </button>
                  </div>
                  <span className="text-xs font-normal font-mono text-muted-foreground">
                    {filteredRecords.length} of {usage.records.length} records
                  </span>
                </div>
              </CardHeader>
              <CardContent className="p-0">
                {/* Model dropdown selector & stats */}
                <div className="flex flex-wrap items-center justify-between gap-3 p-3 border-b border-border/40 bg-card/60 text-xs">
                  <div className="flex flex-wrap items-center gap-2.5">
                    <span className="text-muted-foreground text-[11px] font-medium">Model:</span>
                    <select
                      aria-label="Filter by model"
                      value={selectedModel}
                      onChange={(e) => {
                        setSelectedModel(e.target.value);
                        setUsagePage(1);
                      }}
                      className="h-8 max-w-[280px] truncate rounded-md border border-border/90 bg-background px-2.5 py-1 text-xs font-medium text-foreground shadow-xs transition-colors hover:border-primary/50 focus:outline-none focus:ring-1 focus:ring-primary cursor-pointer font-mono"
                    >
                      <option value="all">
                        All Models ({usage.total_requests} reqs, {formatNumber(usage.total_input_tokens + usage.total_output_tokens)} tok)
                      </option>
                      {Object.entries(usage.by_model).map(([model, stats]) => (
                        <option key={model} value={model}>
                          {model} ({formatNumber(stats.requests)} reqs, {formatNumber(stats.input_tokens + stats.output_tokens)} tok)
                        </option>
                      ))}
                    </select>

                    {selectedModel !== "all" && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSelectedModel("all");
                          setUsagePage(1);
                        }}
                        className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground cursor-pointer"
                      >
                        Reset
                      </Button>
                    )}
                  </div>

                  {activeModelStats && (
                    <div className="flex flex-wrap items-center gap-2 font-mono text-[11px]">
                      <Badge variant="outline">
                        Reqs: <span className="font-semibold ml-1">{formatNumber(activeModelStats.requests)}</span>
                      </Badge>
                      <Badge variant="outline">
                        In: <span className="font-semibold ml-1">{formatNumber(activeModelStats.input_tokens)}</span>
                      </Badge>
                      <Badge variant="outline">
                        Out: <span className="font-semibold ml-1">{formatNumber(activeModelStats.output_tokens)}</span>
                      </Badge>
                      <Badge variant="outline" className="text-emerald-500 border-emerald-500/30">
                        Cache: <span className="font-semibold ml-1">{formatNumber(activeModelStats.cache_read_tokens)}</span>
                      </Badge>
                    </div>
                  )}
                </div>

                {/* Table */}
                <div className="overflow-x-auto scroll-thin max-h-[380px]">
                  <Table>
                    <TableHeader>
                      <TableRow className="text-xs">
                        <TableHead className="w-40">Timestamp</TableHead>
                        <TableHead>Model</TableHead>
                        <TableHead>Endpoint</TableHead>
                        <TableHead className="text-right">Total Input</TableHead>
                        <TableHead className="text-right">Fresh</TableHead>
                        <TableHead className="text-right">Cache Hit</TableHead>
                        <TableHead className="text-right">Output</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody className="text-xs">
                      {filteredRecords.length === 0 ? (
                        <TableRow>
                          <TableCell colSpan={7} className="h-24 text-center text-muted-foreground">
                            No requests found for {selectedModel}.
                          </TableCell>
                        </TableRow>
                      ) : (
                        filteredRecords
                          .slice((usagePage - 1) * usagePageSize, usagePage * usagePageSize)
                          .map((r, idx) => {
                            const freshIn = r.usage?.input_tokens ?? r.usage?.prompt_tokens ?? 0;
                            const cachedIn = r.usage?.cache_read_input_tokens ?? 0;
                            const totalIn = freshIn + cachedIn;
                            const outTok = r.usage?.output_tokens ?? r.usage?.completion_tokens ?? 0;

                            return (
                              <TableRow key={idx}>
                                <TableCell className="font-mono text-muted-foreground whitespace-nowrap">
                                  {formatTimestamp(r.timestamp)}
                                </TableCell>
                                <TableCell className="font-medium">
                                  <Badge variant="outline" className="font-mono text-[10px]">
                                    {r.model}
                                  </Badge>
                                </TableCell>
                                <TableCell className="font-mono text-muted-foreground text-[11px] truncate max-w-[180px]">
                                  {r.endpoint}
                                </TableCell>
                                <TableCell className="text-right font-mono font-medium">
                                  {formatNumber(totalIn)}
                                </TableCell>
                                <TableCell className="text-right font-mono text-muted-foreground">
                                  {formatNumber(freshIn)}
                                </TableCell>
                                <TableCell className="text-right font-mono text-emerald-500 font-medium">
                                  {formatNumber(cachedIn)}
                                </TableCell>
                                <TableCell className="text-right font-mono font-medium">
                                  {formatNumber(outTok)}
                                </TableCell>
                              </TableRow>
                            );
                          })
                      )}
                    </TableBody>
                  </Table>
                </div>

                {/* Pagination */}
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-3 border-t border-border/60 text-xs text-muted-foreground">
                  <div className="flex flex-wrap items-center gap-3">
                    <span className="font-mono text-[11px]">
                      Showing {filteredRecords.length === 0 ? 0 : (usagePage - 1) * usagePageSize + 1}–
                      {Math.min(usagePage * usagePageSize, filteredRecords.length)} of {filteredRecords.length} records
                    </span>
                    <div className="flex items-center gap-1.5 text-[11px]">
                      <span>Page size:</span>
                      {[10, 25, 50].map((size) => (
                        <button
                          key={size}
                          type="button"
                          onClick={() => {
                            setUsagePageSize(size);
                            setUsagePage(1);
                          }}
                          className={`rounded px-1.5 py-0.5 font-mono cursor-pointer transition-colors ${
                            usagePageSize === size
                              ? "bg-primary text-primary-foreground font-semibold"
                              : "bg-muted hover:bg-accent text-foreground"
                          }`}
                        >
                          {size}
                        </button>
                      ))}
                    </div>
                  </div>

                  <div className="flex items-center gap-2 self-end sm:self-auto">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={usagePage <= 1}
                      onClick={() => setUsagePage((p) => Math.max(1, p - 1))}
                      className="h-7 px-2.5 text-xs cursor-pointer"
                    >
                      Previous
                    </Button>
                    <span className="font-mono text-xs px-1">
                      {usagePage} / {totalPages}
                    </span>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={usagePage >= totalPages}
                      onClick={() => setUsagePage((p) => Math.min(totalPages, p + 1))}
                      className="h-7 px-2.5 text-xs cursor-pointer"
                    >
                      Next
                    </Button>
                  </div>
                </div>
              </CardContent>
            </Card>
          );
        })() : (
          <Card className="border-dashed p-6 text-center">
            <p className="text-xs text-muted-foreground">
              No proxy token usage recorded yet. Calls made through the IMC proxy will appear here.
            </p>
          </Card>
        )}
      </div>

      {/* Indexed Repositories */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-lg font-bold tracking-tight text-foreground">Indexed Projects</h2>
            <p className="text-xs text-muted-foreground">
              Repositories indexed on this machine with symbols, edges, and chunks.
            </p>
          </div>
          <Link
            href="/projects"
            className="text-xs text-primary hover:underline font-medium flex items-center gap-1"
          >
            View all projects <ArrowRight className="h-3 w-3" />
          </Link>
        </div>

        {loading ? (
          <div className="flex h-32 items-center justify-center rounded-xl border border-border/60 bg-card">
            <span className="text-xs text-muted-foreground">Loading repositories...</span>
          </div>
        ) : projects.length === 0 ? (
          <Card className="border-dashed p-8 text-center">
            <FolderGit2 className="mx-auto h-8 w-8 text-muted-foreground/60 mb-2" />
            <h3 className="font-semibold text-sm text-foreground">No projects indexed yet</h3>
            <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
              Run <code className="bg-muted px-1.5 py-0.5 rounded font-mono">imc .</code> in your terminal to index the current repository.
            </p>
          </Card>
        ) : (
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {projects.map((p) => {
              const isSelected = selectedPath === p.path;
              return (
                <div
                  key={p.id}
                  className={`glow-card flex flex-col justify-between rounded-xl border bg-card p-5 transition-all ${
                    isSelected ? "border-primary/60 shadow-sm" : "border-border/80"
                  }`}
                >
                  <div className="space-y-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="truncate">
                        <div className="flex items-center gap-2">
                          <span className="font-bold text-sm text-foreground truncate">{p.name}</span>
                          {isSelected && (
                            <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
                              Selected
                            </span>
                          )}
                        </div>
                        <p className="text-[11px] font-mono text-muted-foreground truncate mt-0.5" title={p.path}>
                          {p.path}
                        </p>
                      </div>
                      <Badge variant="secondary" className="font-mono text-[11px] shrink-0">
                        {p.db_size_mb.toFixed(1)} MB
                      </Badge>
                    </div>

                    <div className="grid grid-cols-4 gap-1 rounded-lg bg-muted/30 p-2 text-center text-xs">
                      <div>
                        <div className="font-mono font-semibold text-foreground">{formatNumber(p.counts.files)}</div>
                        <div className="text-[10px] text-muted-foreground">Files</div>
                      </div>
                      <div>
                        <div className="font-mono font-semibold text-foreground">{formatNumber(p.counts.symbols)}</div>
                        <div className="text-[10px] text-muted-foreground">Symbols</div>
                      </div>
                      <div>
                        <div className="font-mono font-semibold text-foreground">{formatNumber(p.counts.edges)}</div>
                        <div className="text-[10px] text-muted-foreground">Edges</div>
                      </div>
                      <div>
                        <div className="font-mono font-semibold text-foreground">{formatNumber(p.counts.chunks)}</div>
                        <div className="text-[10px] text-muted-foreground">Chunks</div>
                      </div>
                    </div>
                  </div>

                  <div className="mt-4 flex items-center gap-2 pt-2 border-t border-border/50">
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedPath(p.path);
                        router.push("/search");
                      }}
                      className="flex-1 rounded-md bg-primary/10 px-3 py-1.5 text-xs font-medium text-primary hover:bg-primary/20 transition-colors text-center cursor-pointer"
                    >
                      Search
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedPath(p.path);
                        router.push("/trace");
                      }}
                      className="flex-1 rounded-md border border-border px-3 py-1.5 text-xs font-medium text-foreground hover:bg-accent transition-colors text-center cursor-pointer"
                    >
                      Call Tree
                    </button>
                    <Link
                      href={`/projects/detail?id=${p.id}`}
                      className="rounded-md border border-border p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                      title="Project Details"
                    >
                      <ExternalLink className="h-3.5 w-3.5" />
                    </Link>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
