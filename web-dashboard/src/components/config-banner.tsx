"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { TriangleAlert, X } from "lucide-react";

interface ConfigStatus {
  complete: boolean;
  missing: string[];
  model: string;
}

const DISMISS_KEY = "imc-config-banner-dismissed";

/**
 * Reminds the user to finish the model configuration before leaning on imc.
 *
 * Shown on every page while the config is incomplete — `llm.model` missing is
 * the most common cause of "model is not available" 404s from the endpoint, and
 * the LLM answer path silently degrades to raw hits. Dismissal lasts for the
 * browser session only.
 */
export function ConfigBanner() {
  const [status, setStatus] = useState<ConfigStatus | null>(null);
  const [dismissed, setDismissed] = useState(true);

  useEffect(() => {
    setDismissed(sessionStorage.getItem(DISMISS_KEY) === "1");
    let cancelled = false;
    fetch("/api/config")
      .then((r) => r.json())
      .then((d) => {
        if (!cancelled && d && !d.error) setStatus(d as ConfigStatus);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  if (!status || status.complete || dismissed) return null;

  const missing = status.missing.length ? status.missing.join(", ") : "settings";

  return (
    <div className="border-b border-amber-500/40 bg-amber-500/10">
      <div className="mx-auto flex max-w-[1500px] items-start gap-3 px-4 py-2.5 sm:px-6 lg:px-8">
        <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
        <div className="flex-1 text-sm">
          <span className="font-medium">Upstream LLM is not fully configured.</span>{" "}
          <span className="text-muted-foreground">
            Missing: <code className="font-mono text-xs">{missing}</code>. Search returns
            raw hits, but LLM reranking and automated reflection will be disabled.
          </span>
        </div>
        <Link
          href="/settings"
          className="shrink-0 rounded-md border border-amber-500/60 px-3 py-1 text-sm font-medium hover:bg-amber-500/20"
        >
          Configure
        </Link>
        <button
          type="button"
          aria-label="Dismiss"
          onClick={() => {
            sessionStorage.setItem(DISMISS_KEY, "1");
            setDismissed(true);
          }}
          className="shrink-0 rounded-md p-1 hover:bg-amber-500/20"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}
