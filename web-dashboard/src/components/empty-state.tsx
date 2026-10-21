"use client";

import { Inbox } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The placeholder every page shows before there is anything to show.
 *
 * A form followed by nothing reads as a broken page: the user cannot tell
 * "not searched yet" from "the request failed". One component keeps the four
 * verbs consistent and gives the empty area a job.
 */
export function EmptyState({
  icon,
  title,
  hint,
  className,
}: {
  icon?: React.ReactNode;
  title: string;
  hint?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-h-[260px] flex-col items-center justify-center rounded-lg border border-dashed border-border bg-muted/20 px-6 py-14 text-center",
        className,
      )}
    >
      <div className="flex h-10 w-10 items-center justify-center rounded-full border border-border bg-background text-muted-foreground">
        {icon ?? <Inbox className="h-5 w-5" />}
      </div>
      <p className="mt-3 text-sm font-medium">{title}</p>
      {hint && (
        <p className="mt-1 max-w-md text-sm text-muted-foreground">{hint}</p>
      )}
    </div>
  );
}
