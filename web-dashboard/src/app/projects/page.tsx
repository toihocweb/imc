"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  FolderGit2,
  Search,
  Network,
  ExternalLink,
  FileCode,
  Code2,
  GitFork,
  Database,
  Check,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useProject } from "@/components/project-context";
import { formatNumber } from "@/lib/format";

export default function ProjectsPage() {
  const router = useRouter();
  const { projects, loading, selectedPath, setSelectedPath } = useProject();
  const [filter, setFilter] = useState("");

  const filtered = projects.filter((p) => {
    const term = filter.toLowerCase();
    return p.name.toLowerCase().includes(term) || p.path.toLowerCase().includes(term);
  });

  return (
    <div className="space-y-6 w-full animate-in fade-in duration-300">
      {/* Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl flex items-center gap-2">
            <FolderGit2 className="h-6 w-6 text-primary" /> Registered Projects
          </h1>
          <p className="text-xs sm:text-sm text-muted-foreground">
            Repositories indexed with tree-sitter AST, call graphs, and vector chunks.
          </p>
        </div>

        {/* Filter Input */}
        <div className="relative w-full sm:w-64">
          <Search className="absolute left-3 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
          <input
            type="text"
            placeholder="Filter projects..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="h-8 w-full rounded-md border border-border bg-card pl-9 pr-3 text-xs text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
          />
        </div>
      </div>

      {/* Projects Grid */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {loading ? (
          <div className="col-span-full flex h-36 items-center justify-center rounded-xl border border-border/80 bg-card">
            <span className="text-xs text-muted-foreground">Loading repositories...</span>
          </div>
        ) : filtered.length === 0 ? (
          <div className="col-span-full rounded-xl border border-dashed border-border p-8 text-center text-xs text-muted-foreground">
            No projects matched the search filter.
          </div>
        ) : (
          filtered.map((p) => {
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

                  <div className="grid grid-cols-4 gap-1 rounded-lg bg-muted/30 p-2.5 text-center text-xs">
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

                <div className="mt-4 flex items-center gap-2 pt-3 border-t border-border/50">
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
                    href={`/projects/${p.id}`}
                    className="rounded-md border border-border p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                    title="View Project Chunks & Details"
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                  </Link>
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* Projects Table */}
      {filtered.length > 0 && (
        <Card className="border-border/80 shadow-xs">
          <CardHeader className="py-3 px-5 border-b border-border/60 bg-muted/20">
            <CardTitle className="text-sm font-semibold flex items-center justify-between">
              <span>All Indexed Repositories</span>
              <span className="text-xs text-muted-foreground font-mono">{filtered.length} total</span>
            </CardTitle>
          </CardHeader>
          <CardContent className="p-0 overflow-x-auto scroll-thin">
            <Table>
              <TableHeader>
                <TableRow className="text-xs">
                  <TableHead>Project</TableHead>
                  <TableHead>Path</TableHead>
                  <TableHead className="text-right">Files</TableHead>
                  <TableHead className="text-right">Symbols</TableHead>
                  <TableHead className="text-right">Edges</TableHead>
                  <TableHead className="text-right">Chunks</TableHead>
                  <TableHead className="text-right">DB Size</TableHead>
                  <TableHead className="w-20 text-center">Action</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody className="text-xs">
                {filtered.map((p) => {
                  const isSelected = selectedPath === p.path;
                  return (
                    <TableRow key={p.id} className="hover:bg-muted/40 transition-colors">
                      <TableCell className="font-semibold text-foreground flex items-center gap-1.5">
                        <FolderGit2 className="h-3.5 w-3.5 text-primary shrink-0" />
                        <span>{p.name}</span>
                        {isSelected && (
                          <span className="h-1.5 w-1.5 rounded-full bg-primary shrink-0" />
                        )}
                      </TableCell>
                      <TableCell className="max-w-[200px] truncate font-mono text-muted-foreground">
                        {p.path}
                      </TableCell>
                      <TableCell className="text-right font-mono">{formatNumber(p.counts.files)}</TableCell>
                      <TableCell className="text-right font-mono">{formatNumber(p.counts.symbols)}</TableCell>
                      <TableCell className="text-right font-mono">{formatNumber(p.counts.edges)}</TableCell>
                      <TableCell className="text-right font-mono">{formatNumber(p.counts.chunks)}</TableCell>
                      <TableCell className="text-right font-mono">{p.db_size_mb.toFixed(1)} MB</TableCell>
                      <TableCell className="text-center">
                        <Link
                          href={`/projects/${p.id}`}
                          className="text-xs text-primary font-medium hover:underline"
                        >
                          Details &rarr;
                        </Link>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
