"use client";

import { useEffect, useState, Suspense } from "react";
import { useSearchParams } from "next/navigation";
import {
  Network,
  ArrowUpRight,
  ArrowDownRight,
  GitFork,
  Copy,
  Check,
  AlertCircle,
  Code2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { useProject } from "@/components/project-context";

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

function TraceContent() {
  const searchParams = useSearchParams();
  const { projects, selectedPath, setSelectedPath } = useProject();

  const [target, setTarget] = useState(searchParams.get("target") || "");
  const [arms, setArms] = useState<TraceArm[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"callees" | "callers" | "both">("callers");
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const executeTrace = async (t: string, r: string, m: "callees" | "callers" | "both") => {
    if (!t.trim() || !r) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/trace", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          target: t.trim(),
          root: r,
          callers: m === "callers",
          both: m === "both",
        }),
      });
      const data = await res.json();
      if (data.results) {
        setArms(data.results);
      } else {
        setError(data.error || "Trace failed to find target symbol or route");
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    const t = searchParams.get("target");
    if (t && selectedPath) {
      setTarget(t);
      executeTrace(t, selectedPath, mode);
    }
  }, [searchParams, selectedPath]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    executeTrace(target, selectedPath, mode);
  };

  const copyText = (val: string, id: string) => {
    navigator.clipboard.writeText(val);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  const getNodeBadgeVariant = (type: string) => {
    const t = type.toLowerCase();
    if (t.includes("route")) return "bg-emerald-500/15 text-emerald-500 border-emerald-500/20";
    if (t.includes("fn") || t.includes("function") || t.includes("method"))
      return "bg-primary/15 text-primary border-primary/20";
    if (t.includes("class") || t.includes("struct") || t.includes("trait"))
      return "bg-purple-500/15 text-purple-500 border-purple-500/20";
    return "bg-muted text-muted-foreground border-border";
  };

  const renderTree = (node: TraceNode, depth = 0): React.ReactNode => {
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

          <span
            className={`rounded px-1.5 py-0.5 text-[10px] font-mono border font-medium ${getNodeBadgeVariant(
              node.type
            )}`}
          >
            {node.type}
          </span>

          <span className="font-mono text-xs sm:text-sm text-foreground truncate max-w-sm sm:max-w-md">
            {node.name}
          </span>

          {node.file && (
            <span className="hidden sm:inline font-mono text-[11px] text-muted-foreground truncate">
              {node.file}:{node.line}
            </span>
          )}

          <button
            type="button"
            onClick={() => copyText(`${node.file}:${node.line}`, `${node.id}-${depth}`)}
            className="opacity-0 group-hover:opacity-100 transition-opacity ml-auto p-1 rounded hover:bg-muted text-muted-foreground hover:text-foreground cursor-pointer"
            title="Copy location"
          >
            {copiedId === `${node.id}-${depth}` ? (
              <Check className="h-3 w-3 text-emerald-500" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
          </button>
        </div>

        {kids.map((k) => renderTree(k, depth + 1))}
      </div>
    );
  };

  return (
    <div className="space-y-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl flex items-center gap-2">
            <Network className="h-6 w-6 text-primary" /> Call Hierarchy & Blast Radius
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Analyze call dependencies, caller impact before renaming/editing, and cross-boundary HTTP route handlers.
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

      {/* Trace Control Card */}
      <Card className="border-border/80 shadow-xs">
        <CardContent className="p-4 sm:p-5">
          <form onSubmit={handleSubmit} className="space-y-3">
            <div className="relative flex items-center">
              <Code2 className="absolute left-3.5 h-4 w-4 text-muted-foreground" />
              <input
                type="text"
                placeholder="Symbol name (e.g. 'authenticate') or HTTP route (e.g. 'GET /api/orders')..."
                value={target}
                onChange={(e) => setTarget(e.target.value)}
                className="h-11 w-full rounded-lg border border-border/90 bg-background pl-10 pr-24 font-mono text-xs sm:text-sm text-foreground transition-all hover:border-primary/50 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/20"
              />
              <Button
                type="submit"
                disabled={loading || !selectedPath}
                className="absolute right-1.5 h-8 px-4 text-xs font-semibold cursor-pointer"
              >
                {loading ? "Tracing..." : "Trace"}
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
                      onClick={() => setMode(item.id as any)}
                      className={`flex items-center gap-1.5 rounded-md px-3 py-1 font-medium transition-colors cursor-pointer ${
                        mode === item.id
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

      {/* Error state */}
      {error && (
        <Card className="border-destructive/40 bg-destructive/10 text-destructive">
          <CardContent className="p-4 text-xs font-medium flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{error}</span>
          </CardContent>
        </Card>
      )}

      {/* Trace Results View */}
      {arms.map((arm, i) => (
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
              <div className="space-y-0.5 min-w-[500px]">{renderTree(arm.tree)}</div>
            ) : (
              <p className="text-xs text-muted-foreground">No call hierarchy found.</p>
            )}
          </CardContent>
        </Card>
      ))}

      {/* Empty prompt */}
      {!loading && arms.length === 0 && !error && (
        <Card className="p-8 text-center border-dashed">
          <Network className="mx-auto h-8 w-8 text-muted-foreground/60 mb-2" />
          <p className="text-sm font-medium text-foreground">No active trace</p>
          <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
            Enter a function name, class method, or an HTTP route like <code className="bg-muted px-1.5 py-0.5 rounded font-mono">GET /api/path</code> to inspect callers and callees.
          </p>
        </Card>
      )}
    </div>
  );
}

export default function TracePage() {
  return (
    <Suspense fallback={<div className="text-xs text-muted-foreground">Loading trace...</div>}>
      <TraceContent />
    </Suspense>
  );
}
