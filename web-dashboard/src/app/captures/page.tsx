"use client";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

interface CaptureInfo {
  id: string;
  path: string;
  size: number;
  modified: number;
  model: string;
  preview: string;
}

interface CaptureStatus {
  enabled: boolean;
  captures: number;
  dir: string;
}

export default function CapturesPage() {
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [captures, setCaptures] = useState<CaptureInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [payload, setPayload] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    try {
      const res = await fetch("/api/captures");
      const d = await res.json();
      if (d.ok) {
        setStatus(d.status);
        setCaptures(d.captures || []);
      } else {
        setError(d.error);
      }
    } catch (e: any) {
      setError(e.message);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const act = async (body: any) => {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/api/captures", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const d = await res.json();
      if (d.ok) {
        setStatus(d.status);
        setCaptures(d.captures || []);
      } else {
        setError(d.error);
      }
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const view = async (id: string) => {
    setSelected(id);
    setPayload(null);
    try {
      const res = await fetch(`/api/captures?id=${encodeURIComponent(id)}`);
      const d = await res.json();
      setPayload(d.ok ? JSON.stringify(d.payload, null, 2) : String(d.error));
    } catch (e: any) {
      setPayload(`error: ${e.message}`);
    }
  };

  const fmtTime = (sec: number) =>
    sec ? new Date(sec * 1000).toLocaleString("vi-VN", { hour12: false }) : "—";

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight sm:text-3xl">Prompt capture</h1>
          <p className="text-sm text-muted-foreground">
            Lưu nguyên payload mà proxy gửi lên model — system prompt, tools, memory block,
            tool result. Bật để xem agent thực sự gửi gì.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant={status?.enabled ? "destructive" : "default"}
            onClick={() => act({ action: status?.enabled ? "off" : "on" })}
            disabled={busy}
          >
            {status?.enabled ? "Tắt capture" : "Bật capture"}
          </Button>
          <Button
            variant="outline"
            onClick={() => act({ action: "clear" })}
            disabled={busy || captures.length === 0}
          >
            Xoá tất cả
          </Button>
        </div>
      </div>

      {error && (
        <Card>
          <CardContent className="pt-6 text-destructive">{error}</CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-lg flex items-center justify-between">
            <span>Trạng thái</span>
            {status?.enabled ? (
              <Badge>đang bật</Badge>
            ) : (
              <Badge variant="secondary">đang tắt</Badge>
            )}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-1 text-sm">
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Đã lưu</span>
            <span className="font-mono">{status?.captures ?? captures.length} payload</span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Thư mục</span>
            <span className="font-mono text-xs break-all text-right">{status?.dir}</span>
          </div>
          <p className="text-xs text-muted-foreground pt-2">
            Tương đương CLI: <code className="font-mono">imc capture on|off|list|show|rm|clear</code>.
            Bật qua web có hiệu lực ngay — proxy đọc trạng thái ở mỗi request.
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Captured prompts ({captures.length})</CardTitle>
        </CardHeader>
        <CardContent>
          {captures.length === 0 ? (
            <p className="text-muted-foreground text-sm">
              Chưa có gì. Bật capture rồi gửi một request qua proxy là sẽ có.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">ID</TableHead>
                  <TableHead className="whitespace-nowrap">Time</TableHead>
                  <TableHead className="whitespace-nowrap">Model</TableHead>
                  <TableHead className="whitespace-nowrap">Size</TableHead>
                  <TableHead>Preview</TableHead>
                  <TableHead className="whitespace-nowrap text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {captures.map((c) => (
                  <TableRow key={c.id}>
                    <TableCell className="font-mono text-xs">{c.id}</TableCell>
                    <TableCell className="whitespace-nowrap text-xs">{fmtTime(c.modified)}</TableCell>
                    <TableCell className="font-mono text-xs">{c.model || "—"}</TableCell>
                    <TableCell className="whitespace-nowrap text-xs">{c.size} B</TableCell>
                    <TableCell className="max-w-[320px] truncate text-xs text-muted-foreground">
                      {c.preview || "—"}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-right">
                      <Button variant="ghost" size="sm" onClick={() => view(c.id)}>
                        View
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => act({ action: "delete", id: c.id })}
                        disabled={busy}
                      >
                        Delete
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {selected && (
        <Card>
          <CardHeader>
            <CardTitle className="text-lg flex items-center justify-between">
              <span>{selected}</span>
              <Button variant="ghost" size="sm" onClick={() => setSelected(null)}>
                Đóng
              </Button>
            </CardTitle>
          </CardHeader>
          <CardContent>
            {payload ? (
              <pre className="max-h-[70vh] overflow-auto whitespace-pre-wrap rounded bg-muted p-3 text-[11px] leading-relaxed">
                {payload}
              </pre>
            ) : (
              <p className="text-sm text-muted-foreground">Đang tải…</p>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
