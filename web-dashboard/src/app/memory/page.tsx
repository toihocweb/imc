"use client";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
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

export default function MemoryPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [root, setRoot] = useState<string>("");
  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [content, setContent] = useState("");
  const [category, setCategory] = useState("note");
  const [memType, setMemType] = useState("work_fact");
  const [scene, setScene] = useState("");
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

  const loadMemories = async (r: string) => {
    if (!r) return;
    try {
      const res = await fetch(`/api/memory?root=${encodeURIComponent(r)}`);
      const data = await res.json();
      setMemories(Array.isArray(data) ? data : []);
    } catch (err: any) {
      setError(err.message);
    }
  };

  useEffect(() => {
    loadMemories(root);
  }, [root]);

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!content.trim() || !root) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/memory", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ root, content, category, memType, scene }),
      });
      const data = await res.json();
      if (data.success) {
        setContent("");
        loadMemories(root);
      } else {
        setError(data.error || "Failed to add memory");
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Memory</h1>

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

      <Card>
        <CardHeader><CardTitle>Add Memory</CardTitle></CardHeader>
        <CardContent>
          <form onSubmit={handleAdd} className="flex w-full max-w-2xl flex-col gap-4">
            <Textarea
              placeholder="Memory content..."
              value={content}
              onChange={(e) => setContent(e.target.value)}
              rows={3}
            />
            <div className="flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center">
              <Select value={category} onValueChange={setCategory}>
                <SelectTrigger className="w-full sm:w-40"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {CATEGORIES.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                </SelectContent>
              </Select>
              <Select value={memType} onValueChange={setMemType}>
                <SelectTrigger className="w-full sm:w-44"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {MEM_TYPES.map((t) => <SelectItem key={t} value={t}>{t}</SelectItem>)}
                </SelectContent>
              </Select>
              <Button type="submit" disabled={loading || !root}>
                {loading ? "Adding..." : "Add Memory"}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {error && <Card><CardContent className="pt-6 text-destructive">{error}</CardContent></Card>}

      <Card>
        <CardHeader><CardTitle>Memories ({memories.length})</CardTitle></CardHeader>
        <CardContent>
          {memories.length === 0 ? (
            <p className="text-muted-foreground">No memories found.</p>
          ) : (
            <div className="space-y-3">
              {memories.map((m) => (
                <div key={m.id} className="border-b pb-3 last:border-0">
                  <div className="flex gap-2 items-center flex-wrap">
                    <Badge variant="secondary">{m.mem_type}</Badge>
                    <Badge variant="outline" className="text-[10px]">{m.category}</Badge>
                    {m.scene && <Badge variant="outline" className="text-[10px]">scene: {m.scene}</Badge>}
                    <span className="text-xs text-muted-foreground">
                      #{m.id} · source={m.source} · used {m.use_count}× · imp {m.importance}
                    </span>
                  </div>
                  <div className="text-sm mt-1">{m.content}</div>
                  {m.paths.length > 0 && (
                    <div className="text-xs font-mono text-muted-foreground mt-1">{m.paths.join(", ")}</div>
                  )}
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
