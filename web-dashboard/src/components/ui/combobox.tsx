"use client";

import * as React from "react";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

interface ComboboxProps {
  /** Currently selected value. */
  value: string;
  /** Called with the picked item. */
  onChange: (value: string) => void;
  /** Options to show; filtered live by the typed text. */
  options: string[];
  placeholder?: string;
  id?: string;
  className?: string;
  emptyLabel?: string;
}

/**
 * Text input with a real dropdown: click (or focus) opens the list, typing
 * filters it, arrows move the active row, Enter picks it, Escape closes.
 *
 * Built for long lists (an endpoint can serve hundreds of models) — the panel
 * scrolls, the active row is highlighted, and the count is shown so the user
 * knows the list is filtered rather than complete.
 */
export function Combobox({
  value,
  onChange,
  options,
  placeholder,
  id,
  className,
  emptyLabel = "No match",
}: ComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [active, setActive] = React.useState(0);
  const wrapRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);

  const filtered = React.useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return options;
    return options.filter((o) => o.toLowerCase().includes(q));
  }, [options, query]);

  // close when clicking outside
  React.useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
        setQuery("");
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  // keep the active row visible
  React.useEffect(() => {
    if (!open || !listRef.current) return;
    const el = listRef.current.querySelector<HTMLElement>(`[data-index="${active}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const pick = (item: string) => {
    onChange(item);
    setQuery("");
    setOpen(false);
    inputRef.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      const dir = e.key === "ArrowDown" ? 1 : -1;
      setActive((i) => Math.min(filtered.length - 1, Math.max(0, i + dir)));
      return;
    }
    if (e.key === "Enter") {
      if (open && filtered[active]) {
        e.preventDefault();
        pick(filtered[active]);
      }
      return;
    }
    if (e.key === "Escape") {
      if (open) {
        e.preventDefault();
        setOpen(false);
        setQuery("");
      }
    }
  };

  return (
    <div ref={wrapRef} className="relative">
      <div className="relative">
        <input
          id={id}
          ref={inputRef}
          value={open ? query : value}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
            if (!open) setOpen(true);
          }}
          onFocus={() => {
            setOpen(true);
            setQuery("");
            setActive(0);
          }}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          autoComplete="off"
          role="combobox"
          aria-expanded={open}
          aria-controls={`${id ?? "combobox"}-list`}
          className={cn(
            "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 pr-9 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
            className
          )}
        />
        <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      </div>

      {open && (
        <div
          ref={listRef}
          id={`${id ?? "combobox"}-list`}
          role="listbox"
          className="absolute z-50 mt-1 max-h-72 w-full overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md"
        >
          {filtered.length === 0 ? (
            <div className="px-3 py-2 text-sm text-muted-foreground">{emptyLabel}</div>
          ) : (
            <>
              <div className="px-3 py-1 text-[11px] text-muted-foreground">
                {filtered.length} / {options.length}
              </div>
              {filtered.map((item, i) => (
                <button
                  key={item}
                  type="button"
                  data-index={i}
                  role="option"
                  aria-selected={item === value}
                  onMouseEnter={() => setActive(i)}
                  onClick={() => pick(item)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-sm px-3 py-1.5 text-left text-sm",
                    i === active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60"
                  )}
                >
                  <Check
                    className={cn("h-3.5 w-3.5 shrink-0", item === value ? "opacity-100" : "opacity-0")}
                  />
                  <span className="truncate font-mono text-xs">{item}</span>
                </button>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
