"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Trash2, Plus, RefreshCw } from "lucide-react";
import { formatNumber } from "@/lib/format";

interface Mappings {
  path: string;
  mappings: Record<string, string>;
}

export default function ModelMapPage() {
  const [state, setState] = useState<Mappings | null>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    try {
      const res = await fetch("/api/model-map");
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "failed to load");
      setState(data);
    } catch (e: any) {
      setError(e.message);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!from.trim() || !to.trim()) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const res = await fetch("/api/model-map", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ from: from.trim(), to: to.trim() }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "failed to save");
      setState(data);
      setFrom("");
      setTo("");
      setMessage(`mapped ${from.trim()} → ${to.trim()}`);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const remove = async (key: string) => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const res = await fetch("/api/model-map", {
        method: "DELETE",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ from: key }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "failed to remove");
      setState(data);
      setMessage(`removed ${key}`);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const rows = Object.entries(state?.mappings || {}).sort(([a], [b]) => a.localeCompare(b));

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Model mapping</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          The proxy rewrites the model a client asks for into the model the upstream serves.
          Unmapped models that look unknown fall back to the default model from the imc config.
        </p>
      </div>

      {state?.path && (
        <p className="break-all text-xs text-muted-foreground">
          config: <code className="font-mono">{state.path}</code>
        </p>
      )}

      {message && (
        <p className="rounded-md border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-300">
          {message}
        </p>
      )}
      {error && (
        <p className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </p>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Add mapping</CardTitle>
          <CardDescription>
            The client asks for the left model, the upstream receives the right one.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={add} className="flex flex-col gap-3 sm:flex-row sm:items-end">
            <div className="flex-1">
              <label className="text-xs text-muted-foreground">Requested model</label>
              <Input
                placeholder="claude-pro-max"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
                className="mt-1 font-mono"
              />
            </div>
            <div className="flex-1">
              <label className="text-xs text-muted-foreground">Upstream model</label>
              <Input
                placeholder="claude-haiku-4.5"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                className="mt-1 font-mono"
              />
            </div>
            <Button type="submit" disabled={busy || !from.trim() || !to.trim()}>
              <Plus className="mr-1.5 h-4 w-4" />
              Add
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0">
          <div>
            <CardTitle className="text-lg">Active mappings</CardTitle>
            <CardDescription>{formatNumber(rows.length)} mapping(s)</CardDescription>
          </div>
          <Button variant="outline" size="sm" onClick={load} disabled={busy}>
            <RefreshCw className="mr-1.5 h-4 w-4" />
            Reload
          </Button>
        </CardHeader>
        <CardContent>
          {rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No mapping yet — every model is forwarded unchanged.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Requested</TableHead>
                  <TableHead>Upstream</TableHead>
                  <TableHead className="w-20 text-right">Remove</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map(([key, value]) => (
                  <TableRow key={key}>
                    <TableCell className="font-mono text-sm">{key}</TableCell>
                    <TableCell className="font-mono text-sm">
                      <Badge variant="secondary" className="font-mono">
                        {value}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => remove(key)}
                        disabled={busy}
                        aria-label={`remove ${key}`}
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
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
