"use client";

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Trash2, Plus, RefreshCw, Pencil, Check, X, Zap, Bot, Sliders } from "lucide-react";
import { formatNumber } from "@/lib/format";

interface Mappings {
  path: string;
  mappings: Record<string, string>;
}

type ProviderType = "hyperspace" | "copilot" | "custom";

const COMMON_CLIENT_MODELS = [
  "claude-3-5-sonnet-20241022",
  "claude-3-7-sonnet",
  "claude-sonnet-4",
  "claude-opus-4",
  "claude-haiku-4.5",
  "claude-pro-max",
  "claude-pro-max-opus[1m]",
  "gpt-4o",
  "gpt-4.1",
  "gpt-5",
];

function parseProvider(value: string): { provider: ProviderType; model: string } {
  if (value.startsWith("hyperspace:")) {
    return { provider: "hyperspace", model: value.slice("hyperspace:".length) };
  }
  if (value.startsWith("copilot:")) {
    return { provider: "copilot", model: value.slice("copilot:".length) };
  }
  if (value.startsWith("anthropic--")) {
    return { provider: "hyperspace", model: value };
  }
  return { provider: "custom", model: value };
}

export default function ModelMapPage() {
  const [state, setState] = useState<Mappings | null>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [selectedProvider, setSelectedProvider] = useState<ProviderType>("hyperspace");
  
  // Available models fetched from endpoints
  const [hyperspaceModels, setHyperspaceModels] = useState<string[]>([]);
  const [copilotModels, setCopilotModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);

  // Edit row state
  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [editFrom, setEditFrom] = useState("");
  const [editTo, setEditTo] = useState("");
  const [editProvider, setEditProvider] = useState<ProviderType>("hyperspace");

  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadMappings = async () => {
    try {
      const res = await fetch("/api/model-map");
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "failed to load");
      setState(data);
    } catch (e: any) {
      setError(e.message);
    }
  };

  const loadAvailableModels = async () => {
    setLoadingModels(true);
    try {
      const [hyperspaceRes, copilotRes] = await Promise.allSettled([
        fetch("/v1/hyperspace-models"),
        fetch("/v1/models"),
      ]);

      if (hyperspaceRes.status === "fulfilled" && hyperspaceRes.value.ok) {
        const data = await hyperspaceRes.value.json();
        const list = (data.data || [])
          .map((m: any) => (typeof m === "string" ? m : m.id))
          .filter(Boolean)
          .sort();
        setHyperspaceModels(list);
      }

      if (copilotRes.status === "fulfilled" && copilotRes.value.ok) {
        const data = await copilotRes.value.json();
        const list = (data.data || [])
          .map((m: any) => (typeof m === "string" ? m : m.id))
          .filter(Boolean)
          .sort();
        setCopilotModels(list);
      }
    } catch (e: any) {
      console.warn("Failed to fetch available models", e);
    } finally {
      setLoadingModels(false);
    }
  };

  useEffect(() => {
    loadMappings();
    loadAvailableModels();
  }, []);

  // When picking a model from the dropdown for the "Add" form
  const handleDropdownSelect = (modelId: string, provider: ProviderType) => {
    if (!modelId) return;
    if (provider === "hyperspace") {
      setTo(`hyperspace:${modelId}`);
    } else if (provider === "copilot") {
      setTo(`copilot:${modelId}`);
    } else {
      setTo(modelId);
    }
  };

  // When picking a model from the dropdown for inline editing
  const handleEditDropdownSelect = (modelId: string, provider: ProviderType) => {
    if (!modelId) return;
    if (provider === "hyperspace") {
      setEditTo(`hyperspace:${modelId}`);
    } else if (provider === "copilot") {
      setEditTo(`copilot:${modelId}`);
    } else {
      setEditTo(modelId);
    }
  };

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
      setMessage(`Mapped ${from.trim()} → ${to.trim()}`);
      setFrom("");
      setTo("");
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const startEdit = (key: string, value: string) => {
    setEditingKey(key);
    setEditFrom(key);
    setEditTo(value);
    const parsed = parseProvider(value);
    setEditProvider(parsed.provider);
  };

  const cancelEdit = () => {
    setEditingKey(null);
    setEditFrom("");
    setEditTo("");
  };

  const saveEdit = async (oldKey: string) => {
    const newFrom = editFrom.trim();
    const newTo = editTo.trim();
    if (!newFrom || !newTo) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      // 1. Update/Add new mapping
      const res = await fetch("/api/model-map", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ from: newFrom, to: newTo }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "failed to save");

      // 2. If key name was changed, delete old key
      if (newFrom !== oldKey) {
        await fetch("/api/model-map", {
          method: "DELETE",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ from: oldKey }),
        });
        const refreshedRes = await fetch("/api/model-map");
        const refreshedData = await refreshedRes.json();
        setState(refreshedData);
      } else {
        setState(data);
      }

      setEditingKey(null);
      setMessage(`Updated ${newFrom} → ${newTo}`);
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
      if (editingKey === key) cancelEdit();
      setMessage(`Removed ${key}`);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setBusy(false);
    }
  };

  const rows = Object.entries(state?.mappings || {}).sort(([a], [b]) => a.localeCompare(b));

  return (
    <div className="w-full space-y-6 animate-in fade-in duration-200">
      <div>
        <h1 className="text-2xl font-bold tracking-tight sm:text-3xl flex items-center gap-2">
          <Sliders className="h-6 w-6 text-primary" /> Model Mapping
        </h1>
        <p className="mt-1 text-sm text-muted-foreground">
          The proxy rewrites the model a client asks for into the model the upstream serves.
          Select from Hyperspace or Copilot catalogs or customize freely.
        </p>
      </div>

      {state?.path && (
        <p className="break-all text-xs text-muted-foreground">
          config: <code className="font-mono bg-muted/50 px-1.5 py-0.5 rounded">{state.path}</code>
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

      {/* Suggested client models datalist */}
      <datalist id="suggested-requested-models">
        {COMMON_CLIENT_MODELS.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>

      {/* Add Mapping Form */}
      <Card className="border-border/80">
        <CardHeader className="pb-3">
          <CardTitle className="text-base font-semibold flex items-center justify-between">
            <span>Add Model Mapping</span>
            {loadingModels && (
              <span className="text-xs text-muted-foreground font-normal flex items-center gap-1">
                <RefreshCw className="h-3 w-3 animate-spin" /> Fetching model catalogs...
              </span>
            )}
          </CardTitle>
          <CardDescription className="text-xs">
            Pick a model from Hyperspace or Copilot to auto-fill the upstream model, and edit as needed.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={add} className="space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {/* Left Column: Requested Model */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  Requested Model <span className="text-muted-foreground font-normal">(what client requests)</span>
                </label>
                <Input
                  list="suggested-requested-models"
                  placeholder="e.g. claude-pro-max, claude-3-7-sonnet"
                  value={from}
                  onChange={(e) => setFrom(e.target.value)}
                  className="font-mono text-xs"
                />
                <p className="text-[11px] text-muted-foreground">
                  Type custom or select from suggestions list.
                </p>
              </div>

              {/* Right Column: Upstream Model Selector */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-foreground">
                    Upstream Model <span className="text-muted-foreground font-normal">(what proxy forwards to)</span>
                  </label>
                  {/* Provider Selector Tabs */}
                  <div className="flex items-center rounded-md border border-border/80 p-0.5 bg-muted/40 text-[11px]">
                    <button
                      type="button"
                      onClick={() => setSelectedProvider("hyperspace")}
                      className={`px-2 py-0.5 rounded transition-all cursor-pointer flex items-center gap-1 ${
                        selectedProvider === "hyperspace"
                          ? "bg-card text-foreground font-semibold shadow-xs"
                          : "text-muted-foreground hover:text-foreground"
                      }`}
                    >
                      <Zap className="h-3 w-3 text-amber-500" />
                      Hyperspace ({hyperspaceModels.length})
                    </button>
                    <button
                      type="button"
                      onClick={() => setSelectedProvider("copilot")}
                      className={`px-2 py-0.5 rounded transition-all cursor-pointer flex items-center gap-1 ${
                        selectedProvider === "copilot"
                          ? "bg-card text-foreground font-semibold shadow-xs"
                          : "text-muted-foreground hover:text-foreground"
                      }`}
                    >
                      <Bot className="h-3 w-3 text-blue-500" />
                      Copilot ({copilotModels.length})
                    </button>
                    <button
                      type="button"
                      onClick={() => setSelectedProvider("custom")}
                      className={`px-2 py-0.5 rounded transition-all cursor-pointer ${
                        selectedProvider === "custom"
                          ? "bg-card text-foreground font-semibold shadow-xs"
                          : "text-muted-foreground hover:text-foreground"
                      }`}
                    >
                      Custom
                    </button>
                  </div>
                </div>

                {/* Dropdown for selected provider */}
                {selectedProvider === "hyperspace" && (
                  <select
                    aria-label="Select Hyperspace Model"
                    onChange={(e) => handleDropdownSelect(e.target.value, "hyperspace")}
                    defaultValue=""
                    className="h-8 w-full rounded-md border border-border bg-card px-2.5 text-xs font-mono text-foreground focus:outline-none focus:ring-1 focus:ring-primary cursor-pointer"
                  >
                    <option value="" disabled>
                      -- Select Hyperspace Model ({hyperspaceModels.length} available) --
                    </option>
                    {hyperspaceModels.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                )}

                {selectedProvider === "copilot" && (
                  <select
                    aria-label="Select Copilot Model"
                    onChange={(e) => handleDropdownSelect(e.target.value, "copilot")}
                    defaultValue=""
                    className="h-8 w-full rounded-md border border-border bg-card px-2.5 text-xs font-mono text-foreground focus:outline-none focus:ring-1 focus:ring-primary cursor-pointer"
                  >
                    <option value="" disabled>
                      -- Select Copilot Model ({copilotModels.length} available) --
                    </option>
                    {copilotModels.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                )}

                {/* Editable Model Input */}
                <Input
                  placeholder="e.g. hyperspace:anthropic--claude-4.6-sonnet or copilot:gpt-4o"
                  value={to}
                  onChange={(e) => setTo(e.target.value)}
                  className="font-mono text-xs"
                />
                <p className="text-[11px] text-muted-foreground">
                  Pick from the dropdown above or freely edit/modify the model string.
                </p>
              </div>
            </div>

            <div className="flex justify-end pt-1">
              <Button type="submit" disabled={busy || !from.trim() || !to.trim()} size="sm">
                <Plus className="mr-1.5 h-4 w-4" />
                Add Mapping
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Active Mappings Table */}
      <Card className="border-border/80">
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
          <div>
            <CardTitle className="text-base font-semibold">Active Mappings</CardTitle>
            <CardDescription className="text-xs">{formatNumber(rows.length)} active route rewrite(s)</CardDescription>
          </div>
          <Button variant="outline" size="sm" onClick={() => { loadMappings(); loadAvailableModels(); }} disabled={busy}>
            <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
            Reload
          </Button>
        </CardHeader>
        <CardContent className="p-0 overflow-x-auto scroll-thin">
          {rows.length === 0 ? (
            <p className="p-6 text-sm text-center text-muted-foreground">
              No mapping configured yet — incoming client models are forwarded as-is.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow className="text-xs">
                  <TableHead className="w-1/3">Requested Model</TableHead>
                  <TableHead className="w-1/2">Forwarded Upstream Model</TableHead>
                  <TableHead className="w-24 text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody className="text-xs">
                {rows.map(([key, value]) => {
                  const isEditing = editingKey === key;
                  const parsed = parseProvider(value);

                  if (isEditing) {
                    return (
                      <TableRow key={key} className="bg-primary/5">
                        <TableCell className="align-top py-3">
                          <Input
                            value={editFrom}
                            onChange={(e) => setEditFrom(e.target.value)}
                            className="h-8 font-mono text-xs"
                            placeholder="Requested model"
                          />
                        </TableCell>
                        <TableCell className="align-top py-3 space-y-2">
                          {/* Mini provider selector for inline editing */}
                          <div className="flex items-center gap-1.5 text-[11px]">
                            <select
                              aria-label="Edit Provider Source"
                              value={editProvider}
                              onChange={(e) => setEditProvider(e.target.value as ProviderType)}
                              className="h-7 rounded border border-border bg-card px-2 text-xs font-medium cursor-pointer"
                            >
                              <option value="hyperspace">⚡ Hyperspace</option>
                              <option value="copilot">🤖 Copilot</option>
                              <option value="custom">Custom</option>
                            </select>

                            {/* Dropdown for inline editing */}
                            {editProvider === "hyperspace" && (
                              <select
                                aria-label="Select Hyperspace Model for editing"
                                onChange={(e) => handleEditDropdownSelect(e.target.value, "hyperspace")}
                                defaultValue=""
                                className="h-7 flex-1 rounded border border-border bg-card px-2 text-xs font-mono cursor-pointer"
                              >
                                <option value="" disabled>-- Pick Hyperspace Model --</option>
                                {hyperspaceModels.map((m) => (
                                  <option key={m} value={m}>{m}</option>
                                ))}
                              </select>
                            )}

                            {editProvider === "copilot" && (
                              <select
                                aria-label="Select Copilot Model for editing"
                                onChange={(e) => handleEditDropdownSelect(e.target.value, "copilot")}
                                defaultValue=""
                                className="h-7 flex-1 rounded border border-border bg-card px-2 text-xs font-mono cursor-pointer"
                              >
                                <option value="" disabled>-- Pick Copilot Model --</option>
                                {copilotModels.map((m) => (
                                  <option key={m} value={m}>{m}</option>
                                ))}
                              </select>
                            )}
                          </div>

                          <Input
                            value={editTo}
                            onChange={(e) => setEditTo(e.target.value)}
                            className="h-8 font-mono text-xs"
                            placeholder="Upstream target model"
                          />
                        </TableCell>
                        <TableCell className="align-top py-3 text-right">
                          <div className="flex items-center justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-emerald-600 hover:text-emerald-700 hover:bg-emerald-50 dark:hover:bg-emerald-950/30"
                              onClick={() => saveEdit(key)}
                              disabled={busy || !editFrom.trim() || !editTo.trim()}
                              title="Save changes"
                            >
                              <Check className="h-4 w-4" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-8 w-8 text-muted-foreground hover:text-foreground"
                              onClick={cancelEdit}
                              disabled={busy}
                              title="Cancel"
                            >
                              <X className="h-4 w-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  }

                  return (
                    <TableRow key={key} className="hover:bg-muted/30 transition-colors">
                      <TableCell className="font-mono font-semibold text-foreground">
                        {key}
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          {parsed.provider === "hyperspace" ? (
                            <Badge variant="outline" className="border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400 text-[10px] font-sans shrink-0">
                              <Zap className="mr-1 h-3 w-3" /> Hyperspace
                            </Badge>
                          ) : parsed.provider === "copilot" ? (
                            <Badge variant="outline" className="border-blue-500/30 bg-blue-500/10 text-blue-600 dark:text-blue-400 text-[10px] font-sans shrink-0">
                              <Bot className="mr-1 h-3 w-3" /> Copilot
                            </Badge>
                          ) : (
                            <Badge variant="outline" className="text-[10px] font-sans shrink-0">
                              Custom
                            </Badge>
                          )}
                          <span className="font-mono text-xs text-foreground font-medium">
                            {parsed.model}
                          </span>
                          <span className="text-[11px] text-muted-foreground font-mono hidden sm:inline">
                            ({value})
                          </span>
                        </div>
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 text-muted-foreground hover:text-foreground hover:bg-muted"
                            onClick={() => startEdit(key, value)}
                            disabled={busy}
                            title={`Edit ${key}`}
                          >
                            <Pencil className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 text-destructive/80 hover:text-destructive hover:bg-destructive/10"
                            onClick={() => remove(key)}
                            disabled={busy}
                            title={`Remove ${key}`}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
