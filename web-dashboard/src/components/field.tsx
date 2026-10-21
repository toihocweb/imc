"use client";

import { cn } from "@/lib/utils";

/**
 * A labelled form control.
 *
 * The label is a real `<label>` bound to the control, so a click focuses the
 * field and a screen reader announces it — a placeholder does neither, and it
 * disappears the moment the user types.
 */
export function Field({
  label,
  htmlFor,
  hint,
  children,
  className,
}: {
  label: string;
  htmlFor?: string;
  hint?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <label
        htmlFor={htmlFor}
        className="block text-sm font-medium text-foreground/80"
      >
        {label}
      </label>
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  );
}
