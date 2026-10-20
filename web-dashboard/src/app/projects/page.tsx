"use client";
import { useEffect, useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

interface Project {
  id: string;
  name: string;
  path: string;
  counts: { files: number; symbols: number; edges: number; chunks: number; embeddings: number };
  db_size_mb: number;
}

export default function ProjectsPage() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch("/api/projects")
      .then((r) => r.json())
      .then((d) => setProjects(Array.isArray(d) ? d : []))
      .catch(() => setProjects([]))
      .finally(() => setLoading(false));
  }, []);

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Projects</h1>
      <Card>
        <CardHeader>
          <CardTitle>Registered Projects ({projects.length})</CardTitle>
        </CardHeader>
        <CardContent>
          {loading ? (
            <p className="text-muted-foreground">Loading...</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">Name</TableHead>
                  <TableHead className="whitespace-nowrap">Path</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Files</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Symbols</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Edges</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Chunks</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Embeddings</TableHead>
                  <TableHead className="whitespace-nowrap text-right">DB MB</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Action</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {projects.map((p) => (
                  <TableRow key={p.id}>
                    <TableCell className="font-medium">{p.name}</TableCell>
                    <TableCell className="max-w-[140px] truncate font-mono text-xs text-muted-foreground sm:max-w-[320px]">{p.path}</TableCell>
                    <TableCell className="text-right font-mono">{p.counts.files}</TableCell>
                    <TableCell className="text-right font-mono">{p.counts.symbols}</TableCell>
                    <TableCell className="text-right font-mono">{p.counts.edges}</TableCell>
                    <TableCell className="text-right font-mono">{p.counts.chunks}</TableCell>
                    <TableCell className="text-right font-mono">{p.counts.embeddings}</TableCell>
                    <TableCell className="text-right font-mono">{p.db_size_mb.toFixed(2)}</TableCell>
                    <TableCell className="text-right">
                      <Link href={`/projects/${p.id}`} className="text-sm text-primary hover:underline">
                        Open
                      </Link>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
