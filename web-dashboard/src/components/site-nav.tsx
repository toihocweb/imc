"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import {
  Boxes,
  LayoutDashboard,
  Search,
  Network,
  Cpu,
  Brain,
  FolderGit2,
  Terminal,
  Settings,
  Sliders,
  ChevronDown,
  Menu,
  X,
  Check,
  Copy,
} from "lucide-react";
import { ThemeToggle } from "@/components/theme-toggle";
import { useProject } from "@/components/project-context";
import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { href: "/", label: "Overview", icon: LayoutDashboard },
  { href: "/search", label: "Search", icon: Search },
  { href: "/trace", label: "Trace", icon: Network },
  { href: "/compression", label: "Compression", icon: Cpu },
  { href: "/memory", label: "Memory", icon: Brain },
  { href: "/projects", label: "Projects", icon: FolderGit2 },
];

const MORE_ITEMS = [
  { href: "/captures", label: "Prompt Captures", icon: Terminal },
  { href: "/model-map", label: "Model Map", icon: Sliders },
  { href: "/settings", label: "Settings", icon: Settings },
];

function isActive(pathname: string, href: string) {
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(href + "/");
}

export function SiteNav() {
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [copiedCli, setCopiedCli] = useState(false);
  const { projects, selectedPath, setSelectedPath } = useProject();

  useEffect(() => {
    setMobileOpen(false);
    setMoreOpen(false);
  }, [pathname]);

  const copyCliCommand = () => {
    navigator.clipboard.writeText("imc claude");
    setCopiedCli(true);
    setTimeout(() => setCopiedCli(false), 2000);
  };

  const isMoreActive = MORE_ITEMS.some((item) => isActive(pathname, item.href));

  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b border-border/80 bg-background/85 backdrop-blur-md">
      <div className="mx-auto flex h-14 max-w-[1500px] items-center justify-between gap-3 px-3 sm:px-6">
        {/* Brand & Active Project Selector */}
        <div className="flex items-center gap-3">
          <Link
            href="/"
            className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors hover:bg-accent/60"
          >
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary/10 text-primary border border-primary/20">
              <Boxes className="h-4 w-4" />
            </div>
            <div className="flex flex-col">
              <div className="flex items-center gap-1.5 leading-none">
                <span className="font-bold text-sm tracking-tight text-foreground">imc</span>
                <span className="rounded bg-primary/15 px-1 py-0.2 text-[10px] font-semibold text-primary">
                  v0.2
                </span>
              </div>
              <span className="text-[10px] text-muted-foreground hidden sm:inline">
                Code Intelligence
              </span>
            </div>
          </Link>

          {/* Project selector dropdown */}
          {projects.length > 0 && (
            <div className="relative hidden lg:block">
              <select
                aria-label="Active Project"
                value={selectedPath}
                onChange={(e) => setSelectedPath(e.target.value)}
                className="h-8 max-w-[260px] truncate rounded-md border border-border/90 bg-card px-2.5 py-1 text-xs font-medium text-foreground shadow-xs transition-colors hover:border-primary/50 focus:outline-none focus:ring-1 focus:ring-primary cursor-pointer"
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.path}>
                    {p.name} ({p.counts.files} files)
                  </option>
                ))}
              </select>
            </div>
          )}

          {/* Proxy status indicator */}
          <div className="hidden xl:flex items-center gap-1.5 rounded-full border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-0.5 text-[11px] font-medium text-emerald-500">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
            <span>Proxy :12345</span>
          </div>
        </div>

        {/* Desktop Primary Navigation */}
        <nav className="hidden md:flex items-center gap-1">
          {NAV_ITEMS.map((item) => {
            const Icon = item.icon;
            const active = isActive(pathname, item.href);
            return (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-all",
                  active
                    ? "bg-primary/15 text-primary font-semibold shadow-2xs"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground"
                )}
              >
                <Icon className={cn("h-3.5 w-3.5", active ? "text-primary" : "text-muted-foreground")} />
                <span>{item.label}</span>
              </Link>
            );
          })}

          {/* More dropdown */}
          <div className="relative">
            <button
              type="button"
              onClick={() => setMoreOpen((v) => !v)}
              className={cn(
                "flex items-center gap-1 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors",
                isMoreActive
                  ? "bg-accent text-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground"
              )}
            >
              <span>More</span>
              <ChevronDown className="h-3 w-3 text-muted-foreground" />
            </button>

            {moreOpen && (
              <div
                className="absolute right-0 mt-1 w-44 rounded-lg border border-border bg-popover p-1 shadow-lg z-50 animate-in fade-in-50 zoom-in-95"
                onMouseLeave={() => setMoreOpen(false)}
              >
                {MORE_ITEMS.map((item) => {
                  const Icon = item.icon;
                  const active = isActive(pathname, item.href);
                  return (
                    <Link
                      key={item.href}
                      href={item.href}
                      className={cn(
                        "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors",
                        active
                          ? "bg-accent text-foreground font-semibold"
                          : "text-muted-foreground hover:bg-accent hover:text-foreground"
                      )}
                    >
                      <Icon className="h-3.5 w-3.5 text-muted-foreground" />
                      <span>{item.label}</span>
                    </Link>
                  );
                })}
              </div>
            )}
          </div>
        </nav>

        {/* Right utilities */}
        <div className="flex items-center gap-2">
          {/* Quick copy command for agent */}
          <button
            type="button"
            onClick={copyCliCommand}
            title="Copy terminal command: imc claude"
            className="hidden sm:inline-flex items-center gap-1.5 rounded-md border border-border/80 bg-card/60 px-2.5 py-1 text-[11px] font-mono text-muted-foreground hover:bg-accent hover:text-foreground transition-colors cursor-pointer"
          >
            {copiedCli ? (
              <>
                <Check className="h-3 w-3 text-emerald-500" />
                <span className="text-emerald-500">Copied!</span>
              </>
            ) : (
              <>
                <Terminal className="h-3 w-3 text-primary" />
                <span>imc claude</span>
                <Copy className="h-2.5 w-2.5 opacity-60 ml-0.5" />
              </>
            )}
          </button>

          <ThemeToggle />

          {/* Mobile hamburger */}
          <button
            type="button"
            onClick={() => setMobileOpen((v) => !v)}
            aria-expanded={mobileOpen}
            aria-label="Toggle navigation"
            className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-input hover:bg-accent md:hidden"
          >
            {mobileOpen ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
          </button>
        </div>
      </div>

      {/* Mobile Menu Panel */}
      {mobileOpen && (
        <div className="border-t border-border bg-background/95 backdrop-blur-md px-4 py-3 md:hidden">
          {projects.length > 0 && (
            <div className="mb-3">
              <label className="text-[11px] font-medium text-muted-foreground mb-1 block">
                Active Project
              </label>
              <select
                value={selectedPath}
                onChange={(e) => setSelectedPath(e.target.value)}
                className="w-full rounded-md border border-border bg-card px-2.5 py-1.5 text-xs font-medium text-foreground"
              >
                {projects.map((p) => (
                  <option key={p.id} value={p.path}>
                    {p.name}
                  </option>
                ))}
              </select>
            </div>
          )}

          <div className="grid grid-cols-2 gap-1 mb-2">
            {[...NAV_ITEMS, ...MORE_ITEMS].map((item) => {
              const Icon = item.icon;
              const active = isActive(pathname, item.href);
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-2.5 py-2 text-xs font-medium",
                    active
                      ? "bg-primary/15 text-primary font-semibold"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground"
                  )}
                >
                  <Icon className="h-3.5 w-3.5" />
                  <span>{item.label}</span>
                </Link>
              );
            })}
          </div>
        </div>
      )}
    </header>
  );
}
