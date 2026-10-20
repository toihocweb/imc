"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { Menu, X, Boxes } from "lucide-react";
import { ThemeToggle } from "@/components/theme-toggle";
import { cn } from "@/lib/utils";

const LINKS = [
  { href: "/", label: "Overview" },
  { href: "/projects", label: "Projects" },
  { href: "/search", label: "Search" },
  { href: "/trace", label: "Trace" },
  { href: "/memory", label: "Memory" },
  { href: "/compression", label: "Compression" },
];

function isActive(pathname: string, href: string) {
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(href + "/");
}

export function SiteNav() {
  const pathname = usePathname();
  const [open, setOpen] = useState(false);

  // close the mobile panel on navigation
  useEffect(() => {
    setOpen(false);
  }, [pathname]);

  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b border-border bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/60">
      <div className="mx-auto flex h-14 max-w-[1400px] items-center gap-2 px-3 sm:px-6">
        <Link href="/" className="flex shrink-0 items-center gap-2 font-semibold">
          <Boxes className="h-5 w-5 text-primary" />
          <span className="text-sm text-muted-foreground">imc</span>
          <span className="hidden xs:inline sm:inline">Dashboard</span>
        </Link>

        {/* desktop nav */}
        <nav className="ml-4 hidden items-center gap-1 md:flex">
          {LINKS.map((l) => (
            <Link
              key={l.href}
              href={l.href}
              className={cn(
                "rounded-md px-3 py-1.5 text-sm font-medium transition-colors hover:bg-accent hover:text-accent-foreground",
                isActive(pathname, l.href)
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground"
              )}
            >
              {l.label}
            </Link>
          ))}
        </nav>

        <div className="ml-auto flex items-center gap-2">
          <ThemeToggle />
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            aria-controls="mobile-nav"
            aria-label={open ? "Close menu" : "Open menu"}
            className="inline-flex h-9 w-9 items-center justify-center rounded-md border border-input hover:bg-accent hover:text-accent-foreground md:hidden"
          >
            {open ? <X className="h-5 w-5" /> : <Menu className="h-5 w-5" />}
          </button>
        </div>
      </div>

      {/* mobile panel */}
      <div
        id="mobile-nav"
        className={cn(
          "overflow-hidden border-t border-border transition-[max-height] duration-200 md:hidden",
          open ? "max-h-96" : "max-h-0 border-t-0"
        )}
      >
        <nav className="mx-auto grid max-w-[1400px] gap-1 px-3 py-3">
          {LINKS.map((l) => (
            <Link
              key={l.href}
              href={l.href}
              className={cn(
                "rounded-md px-3 py-2 text-sm font-medium",
                isActive(pathname, l.href)
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              )}
            >
              {l.label}
            </Link>
          ))}
        </nav>
      </div>
    </header>
  );
}
