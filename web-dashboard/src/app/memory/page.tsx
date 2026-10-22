"use client";

import { useEffect, useState } from "react";
import {
  Brain,
  Plus,
  Trash2,
  Tag,
  FileCode,
  Flame,
  Clock,
  Sparkles,
  Filter,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useProject } from "@/components/project-context";

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
  const { projects, selectedPath, setSelectedPath } = useProject();

  const [memories, setMemories] = useState<MemoryEntry[]>([]);
  const [content, setContent] = useState("");
  const [category, setCategory] = useState("rule");
  const [memType, setMemType] = useState("business_rule");
  const [scene, setScene] = useState("");
  const [filterType, setFilterType] = useState<string>("all");
  const [loading, setLoading] = useState(false);
  const [deletingId, setDeletingId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

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
    if (selectedPath) {
      loadMemories(selectedPath);
    }
  }, [selectedPath]);

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!content.trim() || !selectedPath) return;
    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/memory", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ root: selectedPath, content, category, memType, scene }),
      });
      const data = await res.json();
      if (data.success) {
        setContent("");
        loadMemories(selectedPath);
      } else {
        setError(data.error || "Failed to add memory");
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  };

  const handleDelete = async (id: number) => {
    if (!selectedPath) return;
    if (!window.confirm(`Delete memory #${id}?`)) return;
    setDeletingId(id);
    setError(null);
    try {
      const res = await fetch(`/api/memory?id=${id}&root=${encodeURIComponent(selectedPath)}`, {
        method: "DELETE",
      });
      const data = await res.json();
      if (data.ok || data.deleted) {
        setMemories((prev) => prev.filter((m) => m.id !== id));
      } else {
        setError(data.error || `Failed to delete memory #${id}`);
      }
    } catch (err: any) {
      setError(err.message);
    } finally {
      setDeletingId(null);
    }
  };

  const filteredMemories = memories.filter((m) => {
    if (filterType === "all") return true;
    return m.mem_type === filterType;
  });

  return (
    <div className="space-y-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl flex items-center gap-2">
            <Brain className="h-6 w-6 text-primary" /> Durable Project Memory
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Repo conventions, lessons learned from past debugging sessions, and architectural rules injected into agent turns.
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

      {/* Add Memory Card */}
      <Card className="border-border/80 shadow-xs">
        <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Plus className="h-4 w-4 text-primary" /> Record Durable Lesson or Convention
          </CardTitle>
        </CardHeader>
        <CardContent className="p-5">
          <form onSubmit={handleAdd} className="space-y-4">
            <Textarea
              placeholder="e.g. Always run pnpm build before pushing to master; npm is not configured for this monorepo..."
              value={content}
              onChange={(e) => setContent(e.target.value)}
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
                <Select value={category} onValueChange={setCategory}>
                  <SelectTrigger className="w-32 h-8 text-xs"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {CATEGORIES.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>

              <Button
                type="submit"
                disabled={loading || !selectedPath || !content.trim()}
                className="ml-auto h-8 px-4 text-xs font-semibold cursor-pointer"
              >
                {loading ? "Saving..." : "Save Memory"}
              </Button>
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

      {/* Filter Tabs */}
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
              onClick={() => setFilterType(tab.id)}
              className={`rounded-md px-2.5 py-1 font-medium transition-colors cursor-pointer ${
                filterType === tab.id
                  ? "bg-primary/15 text-primary font-semibold"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground"
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>

        <div className="text-xs text-muted-foreground font-mono">
          {filteredMemories.length} item{filteredMemories.length !== 1 ? "s" : ""}
        </div>
      </div>

      {/* Memories List */}
      <div className="space-y-3">
        {filteredMemories.length === 0 ? (
          <Card className="p-8 text-center border-dashed">
            <Brain className="mx-auto h-8 w-8 text-muted-foreground/60 mb-2" />
            <p className="text-sm font-medium text-foreground">No memories found</p>
            <p className="text-xs text-muted-foreground mt-1 max-w-sm mx-auto">
              Memories are recorded automatically when the proxy detects corrections and mistakes, or manually above.
            </p>
          </Card>
        ) : (
          filteredMemories.map((m) => (
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
                    used {m.use_count}×
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    · imp {m.importance}
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
                disabled={deletingId === m.id}
                onClick={() => handleDelete(m.id)}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
