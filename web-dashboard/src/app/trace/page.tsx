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

export default function TracePage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [root, setRoot] = useState<string>("");
  const [target, setTarget] = useState("");
  const [arms, setArms] = useState<TraceArm[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"callees" | "callers" | "both">("callers");

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

  const handleTrace = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!target.trim() || !root) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/trace", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ target, root, callers: mode === "callers", both: mode === "both" }),
      });
      const data = await res.json();
      if (data.results) setArms(data.results);
      else setError(data.error || "Trace failed");
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  const renderTree = (node: TraceNode, depth = 0): React.ReactNode => {
    const kids = [...(node.callees || []), ...(node.callers || [])];
    return (
      <div key={`${node.id}-${depth}`} style={{ marginLeft: depth * 16 }}>
        <div className="flex items-center gap-2 py-0.5">
          <Badge variant="secondary" className="text-[10px]">{node.type}</Badge>
          <span className="break-all font-mono text-xs sm:text-sm">{node.name}</span>
          <span className="hidden break-all text-xs text-muted-foreground sm:inline">{node.file}:{node.line}</span>
        </div>
        {kids.map((k) => renderTree(k, depth + 1))}
      </div>
    );
  };

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Trace</h1>

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

      <form onSubmit={handleTrace} className="flex w-full max-w-2xl flex-col gap-3">
        <Input
          placeholder="Symbol or route (e.g. AuthController.login)"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
        />
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-1 text-sm">
            <input type="radio" checked={mode === "callees"} onChange={() => setMode("callees")} />
            Callees
          </label>
          <label className="flex items-center gap-1 text-sm">
            <input type="radio" checked={mode === "callers"} onChange={() => setMode("callers")} />
            Callers
          </label>
          <label className="flex items-center gap-1 text-sm">
            <input type="radio" checked={mode === "both"} onChange={() => setMode("both")} />
            Both
          </label>
          <Button type="submit" disabled={loading || !root} className="ml-auto">
            {loading ? "Tracing..." : "Trace"}
          </Button>
        </div>
      </form>

      {error && <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>}

      {arms.map((arm, i) => (
        <Card key={i}>
          <CardHeader><CardTitle className="text-base">{arm.mode}</CardTitle></CardHeader>
          <CardContent className="overflow-x-auto scroll-thin">{renderTree(arm.tree)}</CardContent>
        </Card>
      ))}
    </div>
  );
}
