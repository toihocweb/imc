"use client";

import { useEffect, useState, Suspense } from "react";
import { useSearchParams, useRouter, usePathname } from "next/navigation";
import Link from "next/link";
import {
  Search as SearchIcon,
  Network,
  Cpu,
  Brain,
  FileCode,
  Code2,
  GitFork,
  ArrowUpRight,
  ArrowDownRight,
  Copy,
  Check,
  Eye,
  Trash2,
  Plus,
  Flame,
  Filter,
  Sparkles,
  Database,
  AlertCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useProject } from "@/components/project-context";
import { formatNumber, formatBytes, formatTimestamp } from "@/lib/format";

export type WorkbenchTab = "search" | "trace" | "compression" | "memory";

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

interface MemoryEntry {
  id: number;
  content: string;
  mem_type: string;
  scene: string;
  category: string;
  paths: string[];
  tags: string[];
  importance: number;
  use_count: number;
  created_at: number;
  last_used: number;
  source: string;
}

const MEM_TYPES = ["work_fact", "work_task", "work_method", "work_artifact", "business_rule", "note"];
const CATEGORIES = ["note", "rule", "convention", "work_fact"];

export function Workbench({ initialTab = "search" }: { initialTab?: WorkbenchTab }) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { projects, selectedPath } = useProject();

  // Tab State
  const tabParam = searchParams.get("tab") as WorkbenchTab | null;
  const [activeTab, setActiveTab] = useState<WorkbenchTab>(tabParam || initialTab);

  // Search State
  const [searchQuery, setSearchQuery] = useState(searchParams.get("q") || "");
  const [searchLimit, setSearchLimit] = useState(10);
  const [searchFilter, setSearchFilter] = useState<"all" | "function" | "class" | "doc">("all");
  const [searchResults, setSearchResults] = useState<SearchHit[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [copiedCitationIdx, setCopiedCitationIdx] = useState<number | null>(null);

  // Trace State
  const [traceTarget, setTraceTarget] = useState(searchParams.get("target") || "");
  const [traceArms, setTraceArms] = useState<TraceArm[]>([]);
  const [traceLoading, setTraceLoading] = useState(false);
  const [traceError, setTraceError] = useState<string | null>(null);
  const [traceMode, setTraceMode] = useState<"callees" | "callers" | "both">("callers");
  const [copiedTraceId, setCopiedTraceId] = useState<string | null>(null);

  // Compression State
  const [ccrStats, setCcrStats] = useState<CcrStats | null>(null);
  const [ccrList, setCcrList] = useState<CcrEntry[]>([]);
  const [ccrDeleting, setCcrDeleting] = useState(false);
  const [selectedRecord, setSelectedRecord] = useState<CcrEntry | null>(null);
  const [copiedKind, setCopiedKind] = useState<string | null>(null);

  // Memory State
  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [memContent, setMemContent] = useState("");
  const [memCategory, setMemCategory] = useState("rule");
  const [memType, setMemType] = useState("business_rule");
  const [memScene, setMemScene] = useState("");
  const [memFilter, setMemFilter] = useState("all");
  const [memLoading, setMemLoading] = useState(false);
  const [deletingMemId, setDeletingMemId] = useState<number | null>(null);
  const [memError, setMemError] = useState<string | null>(null);

  // Synchronize Tab from URL
  useEffect(() => {
    if (tabParam && ["search", "trace", "compression", "memory"].includes(tabParam)) {
      setActiveTab(tabParam);
    }
  }, [tabParam]);

  const handleTabChange = (tab: string) => {
    const nextTab = tab as WorkbenchTab;
    setActiveTab(nextTab);
    const params = new URLSearchParams(searchParams.toString());
    params.set("tab", nextTab);
    router.replace(`${pathname}?${params.toString()}`);
  };

  // --- Search Action ---
  const executeSearch = async (q: string, root: string, lim: number) => {
    if (!q.trim() || !root) return;
    setSearchLoading(true);
    setSearchError(null);
    try {
      const res = await fetch("/api/search", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ query: q, root, limit: lim }),
      });
      const data = await res.json();
      if (Array.isArray(data)) {
        setSearchResults(data);
      } else {
        setSearchError(data.error || "Search returned an unexpected response");
      }
    } catch (err: any) {
      setSearchError(err.message);
    } finally {
      setSearchLoading(false);
    }
  };

  // --- Trace Action ---
  const executeTrace = async (t: string, root: string, m: "callees" | "callers" | "both") => {
    if (!t.trim() || !root) return;
    setTraceLoading(true);
    setTraceError(null);
    try {
      const res = await fetch("/api/trace", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          target: t.trim(),
          root,
          callers: m === "callers",
          both: m === "both",
        }),
      });
      const data = await res.json();
      if (data.results) {
        setTraceArms(data.results);
      } else {
        setTraceError(data.error || "Trace found no matching symbols or routes");
      }
    } catch (err: any) {
      setTraceError(err.message);
    } finally {
      setTraceLoading(false);
    }
  };

  // --- Compression Actions ---
  const refreshCcr = async (root: string) => {
    if (!root) return;
    try {
      const [s, l] = await Promise.all([
        fetch(`/api/compression/stats?root=${encodeURIComponent(root)}`).then((x) => x.json()),
        fetch(`/api/compression?root=${encodeURIComponent(root)}&limit=50`).then((x) => x.json()),
      ]);
      if (!s.error) setCcrStats(s);
      setCcrList(Array.isArray(l) ? l : []);
    } catch (err: any) {
      console.error(err);
    }
  };

  const handleDeleteCcr = async (params: { all?: boolean; days?: number; id?: string }) => {
    if (!selectedPath) return;
    if (params.all && !confirm("Delete ALL stored originals?")) return;
    if (params.days && !confirm(`Delete stored originals older than ${params.days} days?`)) return;
    setCcrDeleting(true);
    try {
      const q = new URLSearchParams();
      if (params.all) q.set("all", "true");
      if (params.days) q.set("days", String(params.days));
      if (params.id) q.set("id", params.id);
      await fetch(`/api/compression?${q.toString()}`, { method: "DELETE" });
      await refreshCcr(selectedPath);
    } catch (err: any) {
      console.error(err);
    } finally {
      setCcrDeleting(false);
    }
  };

  // --- Memory Actions ---
  const loadMemories = async (root: string) => {
    if (!root) return;
    try {
      const res = await fetch(`/api/memory?root=${encodeURIComponent(root)}`);
      const data = await res.json();
      setMemories(Array.isArray(data) ? data : []);
    } catch (err: any) {
      setMemError(err.message);
    }
  };

  const handleAddMemory = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!memContent.trim() || !selectedPath) return;
    setMemLoading(true);
    setMemError(null);
    try {
      const res = await fetch("/api/memory", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          root: selectedPath,
          content: memContent,
          category: memCategory,
          memType,
          scene: memScene,
        }),
      });
      const data = await res.json();
      if (data.success) {
        setMemContent("");
        loadMemories(selectedPath);
      } else {
        setMemError(data.error || "Failed to record memory");
      }
    } catch (err: any) {
      setMemError(err.message);
    } finally {
      setMemLoading(false);
    }
  };

  const handleDeleteMemory = async (id: number) => {
    if (!selectedPath) return;
    if (!window.confirm(`Delete memory #${id}?`)) return;
    setDeletingMemId(id);
    try {
      const res = await fetch(`/api/memory?id=${id}&root=${encodeURIComponent(selectedPath)}`, {
        method: "DELETE",
      });
      const data = await res.json();
      if (data.ok || data.deleted) {
        setMemories((prev) => prev.filter((m) => m.id !== id));
      }
    } catch (err: any) {
      setMemError(err.message);
    } finally {
      setDeletingMemId(null);
    }
  };

  // Pre-load on path change
  useEffect(() => {
    if (selectedPath) {
      if (activeTab === "compression") refreshCcr(selectedPath);
      if (activeTab === "memory") loadMemories(selectedPath);
      const q = searchParams.get("q");
      if (q && activeTab === "search") executeSearch(q, selectedPath, searchLimit);
      const targetParam = searchParams.get("target");
      if (targetParam && activeTab === "trace") executeTrace(targetParam, selectedPath, traceMode);
    }
  }, [selectedPath, activeTab]);

  const copyToClipboard = (val: string, kind: string) => {
    navigator.clipboard.writeText(val);
    setCopiedKind(kind);
    setTimeout(() => setCopiedKind(null), 2000);
  };

  // Quick switch from search to trace
  const jumpToTrace = (sym: string) => {
    setTraceTarget(sym);
    handleTabChange("trace");
    if (selectedPath) {
      executeTrace(sym, selectedPath, "callers");
    }
  };

  const filteredSearchResults = searchResults.filter((hit) => {
    if (searchFilter === "all") return true;
    if (searchFilter === "function") {
      const k = hit.sym_kind?.toLowerCase() || "";
      return k.includes("function") || k.includes("method");
    }
    if (searchFilter === "class") {
      const k = hit.sym_kind?.toLowerCase() || "";
      return k.includes("class") || k.includes("struct") || k.includes("interface");
    }
    if (searchFilter === "doc") {
      return hit.file.endsWith(".md") || hit.file.endsWith(".txt");
    }
    return true;
  });

  const renderTraceTree = (node: TraceNode, depth = 0): React.ReactNode => {
    const kids = [...(node.callees || []), ...(node.callers || [])];
    const isRoot = depth === 0;

    return (
      <div key={`${node.id}-${depth}`} className="relative">
        <div
          className={`group flex items-center gap-2 py-1.5 px-2 rounded-md transition-colors hover:bg-accent/40 ${
            isRoot ? "bg-primary/5 font-semibold border border-primary/20" : ""
          }`}
          style={{ marginLeft: `${depth * 20}px` }}
        >
          {depth > 0 && (
            <span className="text-muted-foreground/40 font-mono text-xs select-none">└──</span>
          )}

          <Badge variant="secondary" className="text-[10px] font-mono">
            {node.type}
          </Badge>

          <span className="font-mono text-xs sm:text-sm text-foreground truncate max-w-sm sm:max-w-md">
            {node.name}
          </span>

          {node.file && (
            <span className="hidden sm:inline font-mono text-[11px] text-muted-foreground truncate">
              {node.file}:{formatNumber(node.line)}
            </span>
          )}

          <button
            type="button"
            onClick={() => {
              navigator.clipboard.writeText(`${node.file}:${node.line}`);
              setCopiedTraceId(`${node.id}-${depth}`);
              setTimeout(() => setCopiedTraceId(null), 2000);
            }}
            className="opacity-0 group-hover:opacity-100 transition-opacity ml-auto p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground cursor-pointer"
            title="Copy location"
          >
            {copiedTraceId === `${node.id}-${depth}` ? (
              <Check className="h-3 w-3 text-emerald-500" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
          </button>
        </div>

        {kids.map((k) => renderTraceTree(k, depth + 1))}
      </div>
    );
  };

  return (
    <div className="w-full space-y-6 animate-in fade-in duration-200">
      {/* Top Header */}
      <div className="border-b border-border/80 pb-4">
        <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
          Code Intelligence Studio
        </h1>
        <p className="text-xs sm:text-sm text-muted-foreground mt-1">
          Unified workspace for retrieval, call-tree tracing, context compression, and durable memory.
        </p>
      </div>

      {/* Primary Unified Tabs */}
      <Tabs value={activeTab} onValueChange={handleTabChange} className="space-y-6">
        <TabsList className="grid grid-cols-2 sm:grid-cols-4 w-full h-auto p-1 bg-muted/60 border border-border rounded-xl">
          <TabsTrigger
            value="search"
            className="flex items-center justify-center gap-2 py-2.5 text-xs sm:text-sm font-semibold cursor-pointer data-[state=active]:shadow-xs"
          >
            <SearchIcon className="h-4 w-4" />
            <span>Search</span>
          </TabsTrigger>
          <TabsTrigger
            value="trace"
            className="flex items-center justify-center gap-2 py-2.5 text-xs sm:text-sm font-semibold cursor-pointer data-[state=active]:shadow-xs"
          >
            <Network className="h-4 w-4" />
            <span>Call Tree & Trace</span>
          </TabsTrigger>
          <TabsTrigger
            value="compression"
            className="flex items-center justify-center gap-2 py-2.5 text-xs sm:text-sm font-semibold cursor-pointer data-[state=active]:shadow-xs"
          >
            <Cpu className="h-4 w-4" />
            <span>Compression & CCR</span>
          </TabsTrigger>
          <TabsTrigger
            value="memory"
            className="flex items-center justify-center gap-2 py-2.5 text-xs sm:text-sm font-semibold cursor-pointer data-[state=active]:shadow-xs"
          >
            <Brain className="h-4 w-4" />
            <span>Project Memory</span>
          </TabsTrigger>
        </TabsList>

        {/* ------------------------------------------------------------- */}
        {/* TAB 1: CODE SEARCH */}
        {/* ------------------------------------------------------------- */}
        <TabsContent value="search" className="space-y-6 m-0 animate-in fade-in-50 duration-200">
          <Card className="border-border/80 shadow-xs">
            <CardContent className="p-4 sm:p-5">
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  executeSearch(searchQuery, selectedPath, searchLimit);
                }}
                className="space-y-3"
              >
                <div className="relative flex items-center">
                  <SearchIcon className="absolute left-3.5 h-4 w-4 text-muted-foreground" />
                  <input
                    type="text"
                    placeholder="Search by natural query or exact symbol (e.g. 'authentication flow', 'verify_token')..."
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    className="h-11 w-full rounded-lg border border-border/90 bg-background pl-10 pr-24 text-sm text-foreground transition-all hover:border-primary/50 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/20"
                  />
                  <Button
                    type="submit"
                    disabled={searchLoading || !selectedPath}
                    className="absolute right-1.5 h-8 px-4 text-xs font-semibold cursor-pointer"
                  >
                    {searchLoading ? "Searching..." : "Search"}
                  </Button>
                </div>

                <div className="flex flex-wrap items-center justify-between gap-2 pt-1 text-xs text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <span>Result limit:</span>
                    {[10, 20, 30].map((n) => (
                      <button
                        key={n}
                        type="button"
                        onClick={() => setSearchLimit(n)}
                        className={`rounded px-2 py-0.5 text-xs font-medium transition-colors cursor-pointer ${
                          searchLimit === n
                            ? "bg-primary text-primary-foreground font-semibold"
                            : "bg-muted hover:bg-accent text-muted-foreground"
                        }`}
                      >
                        {formatNumber(n)}
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

          {searchError && (
            <Card className="border-destructive/40 bg-destructive/10 text-destructive">
              <CardContent className="p-4 text-xs font-medium">{searchError}</CardContent>
            </Card>
          )}

          {searchResults.length > 0 && (
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pt-1">
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
                    onClick={() => setSearchFilter(key)}
                    className={`rounded-md px-2.5 py-1 font-medium transition-colors cursor-pointer ${
                      searchFilter === key
                        ? "bg-primary/15 text-primary font-semibold"
                        : "text-muted-foreground hover:bg-accent hover:text-foreground"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>

              <div className="text-xs text-muted-foreground font-mono">
                Showing {formatNumber(filteredSearchResults.length)} of {formatNumber(searchResults.length)} hits
              </div>
            </div>
          )}

          <div className="space-y-4">
            {filteredSearchResults.map((hit, idx) => {
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
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/20 px-4 py-2.5 text-xs">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="flex h-5 w-5 items-center justify-center rounded bg-primary/10 text-primary font-mono text-[10px] font-bold">
                        #{formatNumber(idx + 1)}
                      </span>
                      <FileCode className="h-4 w-4 text-muted-foreground shrink-0" />
                      <span className="font-mono font-medium text-foreground truncate">
                        {hit.file}
                      </span>
                      <span className="font-mono text-muted-foreground shrink-0">
                        :{formatNumber(hit.start_line)}-{formatNumber(hit.end_line)}
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

                  <pre className="max-h-80 overflow-auto bg-background/60 p-4 font-mono text-xs leading-relaxed text-foreground/90 scroll-thin select-text">
                    {mainText}
                  </pre>

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
                        <button
                          type="button"
                          onClick={() => jumpToTrace(hit.sym_name!)}
                          className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline cursor-pointer"
                        >
                          <Network className="h-3 w-3" /> Trace Callers
                        </button>
                      )}
                      <button
                        type="button"
                        onClick={() => {
                          navigator.clipboard.writeText(`${hit.file}:${hit.start_line}-${hit.end_line}`);
                          setCopiedCitationIdx(idx);
                          setTimeout(() => setCopiedCitationIdx(null), 2000);
                        }}
                        className="inline-flex items-center gap-1 rounded border border-border px-2 py-1 text-[11px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground transition-colors cursor-pointer"
                      >
                        {copiedCitationIdx === idx ? (
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

          {!searchLoading && searchResults.length === 0 && searchQuery && (
            <Card className="p-8 text-center border-dashed">
              <p className="text-sm font-medium text-foreground">No matching code chunks found</p>
              <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
                Try broader keywords or inspect outline with <code className="font-mono bg-muted px-1 rounded">imc read</code>.
              </p>
            </Card>
          )}
        </TabsContent>

        {/* ------------------------------------------------------------- */}
        {/* TAB 2: CALL TREE & TRACE */}
        {/* ------------------------------------------------------------- */}
        <TabsContent value="trace" className="space-y-6 m-0 animate-in fade-in-50 duration-200">
          <Card className="border-border/80 shadow-xs">
            <CardContent className="p-4 sm:p-5">
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  executeTrace(traceTarget, selectedPath, traceMode);
                }}
                className="space-y-3"
              >
                <div className="relative flex items-center">
                  <Code2 className="absolute left-3.5 h-4 w-4 text-muted-foreground" />
                  <input
                    type="text"
                    placeholder="Symbol name (e.g. 'authenticate') or HTTP route (e.g. 'GET /api/orders')..."
                    value={traceTarget}
                    onChange={(e) => setTraceTarget(e.target.value)}
                    className="h-11 w-full rounded-lg border border-border/90 bg-background pl-10 pr-24 font-mono text-xs sm:text-sm text-foreground transition-all hover:border-primary/50 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/20"
                  />
                  <Button
                    type="submit"
                    disabled={traceLoading || !selectedPath}
                    className="absolute right-1.5 h-8 px-4 text-xs font-semibold cursor-pointer"
                  >
                    {traceLoading ? "Tracing..." : "Trace"}
                  </Button>
                </div>

                <div className="flex flex-wrap items-center justify-between gap-3 pt-1 text-xs">
                  <div className="flex items-center gap-1.5 rounded-lg border border-border bg-muted/40 p-1">
                    {[
                      { id: "callers", label: "Callers (Blast Radius)", icon: ArrowUpRight },
                      { id: "callees", label: "Callees (Flow)", icon: ArrowDownRight },
                      { id: "both", label: "Both Directions", icon: GitFork },
                    ].map((item) => {
                      const Icon = item.icon;
                      return (
                        <button
                          key={item.id}
                          type="button"
                          onClick={() => setTraceMode(item.id as any)}
                          className={`flex items-center gap-1.5 rounded-md px-3 py-1 font-medium transition-colors cursor-pointer ${
                            traceMode === item.id
                              ? "bg-primary text-primary-foreground font-semibold shadow-xs"
                              : "text-muted-foreground hover:bg-accent hover:text-foreground"
                          }`}
                        >
                          <Icon className="h-3 w-3" />
                          <span>{item.label}</span>
                        </button>
                      );
                    })}
                  </div>

                  <span className="text-[11px] text-muted-foreground font-mono">
                    Depth limit: 3 levels
                  </span>
                </div>
              </form>
            </CardContent>
          </Card>

          {traceError && (
            <Card className="border-destructive/40 bg-destructive/10 text-destructive">
              <CardContent className="p-4 text-xs font-medium flex items-center gap-2">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{traceError}</span>
              </CardContent>
            </Card>
          )}

          {traceArms.map((arm, i) => (
            <Card key={i} className="border-border/80 shadow-xs">
              <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
                <CardTitle className="text-xs sm:text-sm font-semibold flex items-center justify-between">
                  <span className="flex items-center gap-2 capitalize">
                    <Network className="h-4 w-4 text-primary" />
                    {arm.mode} Call Tree
                  </span>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    Root: {arm.tree?.name}
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent className="p-4 overflow-x-auto scroll-thin">
                {arm.tree ? (
                  <div className="space-y-0.5 min-w-[500px]">{renderTraceTree(arm.tree)}</div>
                ) : (
                  <p className="text-xs text-muted-foreground">No call hierarchy found.</p>
                )}
              </CardContent>
            </Card>
          ))}
        </TabsContent>

        {/* ------------------------------------------------------------- */}
        {/* TAB 3: CONTEXT COMPRESSION & CCR */}
        {/* ------------------------------------------------------------- */}
        <TabsContent value="compression" className="space-y-6 m-0 animate-in fade-in-50 duration-200">
          <div className="grid grid-cols-2 gap-3 sm:gap-4 lg:grid-cols-4">
            {[
              { label: "Cached Originals", val: formatNumber(ccrStats?.records), sub: "Stored in CCR SQLite" },
              { label: "Original Tokens", val: formatNumber(ccrStats?.original_tokens), sub: "Pre-compression" },
              { label: "Compressed Tokens", val: formatNumber(ccrStats?.compressed_tokens), sub: "Forwarded to LLM" },
              {
                label: "Tokens Saved",
                val: formatNumber(ccrStats?.saved_tokens),
                sub: ccrStats ? `${((1 - ccrStats.compression_ratio) * 100).toFixed(1)}% reduction` : "0%",
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

          <Card className="border-border/80 shadow-xs">
            <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                <CardTitle className="text-sm font-semibold flex items-center gap-2">
                  <Database className="h-4 w-4 text-primary" /> Stored Originals ({formatNumber(ccrList.length)})
                  {ccrStats && (
                    <span className="text-xs text-muted-foreground font-mono font-normal ml-1">
                      ({formatBytes(ccrStats.db_bytes)})
                    </span>
                  )}
                </CardTitle>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={ccrDeleting || ccrList.length === 0}
                    onClick={() => handleDeleteCcr({ days: 7 })}
                    className="h-7 px-2.5 text-[11px] font-medium text-muted-foreground hover:text-foreground cursor-pointer"
                    title="Delete records older than 7 days"
                  >
                    Prune &gt;7d
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={ccrDeleting || ccrList.length === 0}
                    onClick={() => handleDeleteCcr({ days: 1 })}
                    className="h-7 px-2.5 text-[11px] font-medium text-muted-foreground hover:text-foreground cursor-pointer"
                    title="Delete records older than 1 day"
                  >
                    Prune &gt;1d
                  </Button>
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={ccrDeleting || ccrList.length === 0}
                    onClick={() => handleDeleteCcr({ all: true })}
                    className="h-7 px-2.5 text-[11px] font-medium cursor-pointer"
                    title="Delete all stored originals"
                  >
                    Clear All
                  </Button>
                </div>
              </div>
            </CardHeader>
            <CardContent className="p-0 overflow-x-auto scroll-thin">
              <Table>
                <TableHeader>
                  <TableRow className="text-xs">
                    <TableHead className="w-28">Handle ID</TableHead>
                    <TableHead>Type</TableHead>
                    <TableHead className="text-right">Original</TableHead>
                    <TableHead className="text-right">Compressed</TableHead>
                    <TableHead className="text-right">Reduction</TableHead>
                    <TableHead className="text-right">Recorded</TableHead>
                    <TableHead className="w-20 text-center">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody className="text-xs">
                  {ccrList.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={7} className="h-24 text-center text-muted-foreground">
                        No CCR records stored for this repository yet.
                      </TableCell>
                    </TableRow>
                  ) : (
                    ccrList.map((e) => {
                      const reduction = e.orig_tokens > 0 ? (((e.orig_tokens - e.comp_tokens) / e.orig_tokens) * 100).toFixed(1) + "%" : "0%";
                      return (
                        <TableRow key={e.id} className="hover:bg-muted/40 transition-colors">
                          <TableCell className="font-mono font-medium text-foreground">
                            #{e.id}
                          </TableCell>
                          <TableCell>
                            <Badge variant="secondary" className="font-mono text-[10px]">
                              {e.kind}
                            </Badge>
                          </TableCell>
                          <TableCell className="text-right font-mono">{formatNumber(e.orig_tokens)} tok</TableCell>
                          <TableCell className="text-right font-mono">{formatNumber(e.comp_tokens)} tok</TableCell>
                          <TableCell className="text-right font-mono font-semibold text-emerald-500">
                            {reduction}
                          </TableCell>
                          <TableCell className="text-right text-muted-foreground text-[11px] font-mono whitespace-nowrap">
                            {formatTimestamp(e.created_at)}
                          </TableCell>
                          <TableCell className="text-center">
                            <div className="flex items-center justify-center gap-1">
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-7 w-7 p-0 text-muted-foreground hover:text-foreground cursor-pointer"
                                title="View Details"
                                onClick={() => setSelectedRecord(e)}
                              >
                                <Eye className="h-3.5 w-3.5" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="sm"
                                disabled={ccrDeleting}
                                className="h-7 w-7 p-0 text-muted-foreground hover:text-destructive cursor-pointer"
                                title="Delete record"
                                onClick={() => handleDeleteCcr({ id: e.id })}
                              >
                                <Trash2 className="h-3.5 w-3.5" />
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        </TabsContent>

        {/* ------------------------------------------------------------- */}
        {/* TAB 4: PROJECT MEMORY */}
        {/* ------------------------------------------------------------- */}
        <TabsContent value="memory" className="space-y-6 m-0 animate-in fade-in-50 duration-200">
          <Card className="border-border/80 shadow-xs">
            <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <Plus className="h-4 w-4 text-primary" /> Record Durable Convention or Lesson
              </CardTitle>
            </CardHeader>
            <CardContent className="p-5">
              <form onSubmit={handleAddMemory} className="space-y-4">
                <Textarea
                  placeholder="e.g. Always run pnpm build before pushing to master; npm is not configured for this monorepo..."
                  value={memContent}
                  onChange={(e) => setMemContent(e.target.value)}
                  rows={3}
                  className="text-xs sm:text-sm leading-relaxed"
                />
                <div className="flex flex-wrap items-center gap-3">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">Type:</span>
                    <Select value={memType} onValueChange={setMemType}>
                      <SelectTrigger className="w-36 h-8 text-xs"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {MEM_TYPES.map((t) => <SelectItem key={t} value={t}>{t}</SelectItem>)}
                      </SelectContent>
                    </Select>
                  </div>

                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">Category:</span>
                    <Select value={memCategory} onValueChange={setMemCategory}>
                      <SelectTrigger className="w-32 h-8 text-xs"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {CATEGORIES.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                      </SelectContent>
                    </Select>
                  </div>

                  <Button
                    type="submit"
                    disabled={memLoading || !selectedPath || !memContent.trim()}
                    className="ml-auto h-8 px-4 text-xs font-semibold cursor-pointer"
                  >
                    {memLoading ? "Saving..." : "Save Memory"}
                  </Button>
                </div>
              </form>
            </CardContent>
          </Card>

          {memError && (
            <Card className="border-destructive/40 bg-destructive/10 text-destructive">
              <CardContent className="p-4 text-xs font-medium">{memError}</CardContent>
            </Card>
          )}

          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pt-1">
            <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-border bg-card p-1 text-xs">
              <Filter className="h-3.5 w-3.5 text-muted-foreground ml-1.5 mr-0.5" />
              {[
                { id: "all", label: "All Types" },
                { id: "business_rule", label: "Business Rules" },
                { id: "work_fact", label: "Work Facts" },
                { id: "work_method", label: "Work Methods" },
                { id: "note", label: "Notes" },
              ].map((tab) => (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setMemFilter(tab.id)}
                  className={`rounded-md px-2.5 py-1 font-medium transition-colors cursor-pointer ${
                    memFilter === tab.id
                      ? "bg-primary/15 text-primary font-semibold"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground"
                  }`}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            <div className="text-xs text-muted-foreground font-mono">
              {formatNumber(memories.filter((m) => memFilter === "all" || m.mem_type === memFilter).length)} memories
            </div>
          </div>

          <div className="space-y-3">
            {memories
              .filter((m) => memFilter === "all" || m.mem_type === memFilter)
              .map((m) => (
                <div
                  key={m.id}
                  className="glow-card group rounded-xl border border-border/80 bg-card p-4 transition-all hover:bg-card/90 flex items-start justify-between gap-4"
                >
                  <div className="space-y-2 flex-1 min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="secondary" className="font-mono text-[11px] font-semibold">
                        {m.mem_type}
                      </Badge>
                      <Badge variant="outline" className="text-[10px] text-muted-foreground">
                        {m.category}
                      </Badge>
                      {m.scene && (
                        <Badge variant="outline" className="text-[10px] text-muted-foreground">
                          scene: {m.scene}
                        </Badge>
                      )}
                      <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
                        <Flame className="h-3 w-3 text-amber-500" />
                        used {formatNumber(m.use_count)}×
                      </span>
                      <span className="text-[11px] text-muted-foreground font-mono">
                        · imp {m.importance.toFixed(2)}
                      </span>
                      <span className="text-[11px] text-muted-foreground ml-auto font-mono">
                        #{m.id}
                      </span>
                    </div>

                    <div className="text-sm text-foreground leading-relaxed break-words font-medium">
                      {m.content}
                    </div>

                    {m.paths.length > 0 && (
                      <div className="flex flex-wrap items-center gap-1.5 pt-1 text-[11px] text-muted-foreground font-mono">
                        <FileCode className="h-3 w-3 shrink-0" />
                        {m.paths.map((p) => (
                          <span key={p} className="rounded bg-muted px-1.5 py-0.5 truncate max-w-xs">
                            {p}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>

                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 w-8 p-0 text-muted-foreground hover:text-destructive hover:bg-destructive/10 shrink-0 cursor-pointer"
                    title={`Delete memory #${m.id}`}
                    aria-label={`Delete memory #${m.id}`}
                    disabled={deletingMemId === m.id}
                    onClick={() => handleDeleteMemory(m.id)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              ))}
          </div>
        </TabsContent>
      </Tabs>

      {/* Details Modal for CCR Record */}
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
                  <Badge variant="secondary" className="font-mono text-xs">
                    {selectedRecord.kind}
                  </Badge>
                </div>
                {selectedRecord.created_at ? (
                  <span className="text-xs text-muted-foreground font-mono">
                    {new Date(selectedRecord.created_at * 1000).toLocaleString()}
                  </span>
                ) : null}
              </div>

              <div className="grid grid-cols-3 gap-3">
                <div className="p-3 bg-muted/40 border border-border/60 rounded-lg">
                  <div className="text-[11px] text-muted-foreground">Original Tokens</div>
                  <div className="text-sm font-mono font-bold mt-0.5">{formatNumber(selectedRecord.orig_tokens)} tok</div>
                </div>
                <div className="p-3 bg-muted/40 border border-border/60 rounded-lg">
                  <div className="text-[11px] text-muted-foreground">Forwarded to Model</div>
                  <div className="text-sm font-mono font-bold mt-0.5">{formatNumber(selectedRecord.comp_tokens)} tok</div>
                </div>
                <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 rounded-lg">
                  <div className="text-[11px] text-emerald-600 dark:text-emerald-400">Tokens Saved</div>
                  <div className="text-sm font-mono font-bold text-emerald-600 dark:text-emerald-400 mt-0.5">
                    {formatNumber(selectedRecord.orig_tokens - selectedRecord.comp_tokens)} tok ({selectedRecord.orig_tokens > 0 ? (((selectedRecord.orig_tokens - selectedRecord.comp_tokens) / selectedRecord.orig_tokens) * 100).toFixed(1) + "%" : "0%"})
                  </div>
                </div>
              </div>

              <Tabs defaultValue="diff" className="flex-1 flex flex-col overflow-hidden">
                <TabsList className="w-fit">
                  <TabsTrigger value="diff">Side-by-Side</TabsTrigger>
                  <TabsTrigger value="compressed">Compressed (Sent to Model)</TabsTrigger>
                  <TabsTrigger value="original">Original (Full)</TabsTrigger>
                </TabsList>

                <TabsContent value="diff" className="flex-1 overflow-hidden mt-3">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3 h-[45vh]">
                    <div className="flex flex-col border border-border rounded-lg overflow-hidden">
                      <div className="bg-muted/40 px-3 py-1.5 text-[11px] font-medium text-muted-foreground border-b border-border flex justify-between">
                        <span>Original Payload</span>
                        <span className="font-mono">{formatNumber(selectedRecord.original?.length)} chars</span>
                      </div>
                      <pre className="flex-1 overflow-auto p-3 text-xs font-mono leading-relaxed bg-background/50 scroll-thin">
                        {selectedRecord.original || "(empty)"}
                      </pre>
                    </div>

                    <div className="flex flex-col border border-border rounded-lg overflow-hidden">
                      <div className="bg-muted/40 px-3 py-1.5 text-[11px] font-medium text-muted-foreground border-b border-border flex justify-between">
                        <span>Compressed Summary</span>
                        <span className="font-mono">{formatNumber(selectedRecord.compressed?.length)} chars</span>
                      </div>
                      <pre className="flex-1 overflow-auto p-3 text-xs font-mono leading-relaxed bg-background/50 scroll-thin">
                        {selectedRecord.compressed || "(empty)"}
                      </pre>
                    </div>
                  </div>
                </TabsContent>

                <TabsContent value="compressed" className="flex-1 flex flex-col overflow-hidden mt-3">
                  <div className="flex justify-between items-center mb-1.5 text-xs text-muted-foreground">
                    <span className="font-mono">{formatNumber(selectedRecord.compressed?.length)} characters</span>
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
                    <span className="font-mono">{formatNumber(selectedRecord.original?.length)} characters</span>
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

function StudioFallback() {
  return (
    <div className="w-full space-y-6 animate-in fade-in duration-200">
      <div className="border-b border-border/80 pb-4">
        <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
          Code Intelligence Studio
        </h1>
        <p className="text-xs sm:text-sm text-muted-foreground mt-1">
          Unified workspace for retrieval, call-tree tracing, context compression, and durable memory.
        </p>
      </div>
      <div className="h-11 w-full rounded-xl bg-muted/60 border border-border animate-pulse" />
      <div className="h-44 w-full rounded-xl bg-card border border-border/80 animate-pulse" />
    </div>
  );
}

export default function WorkbenchPage({ defaultTab }: { defaultTab?: WorkbenchTab }) {
  return (
    <Suspense fallback={<StudioFallback />}>
      <Workbench initialTab={defaultTab} />
    </Suspense>
  );
}
