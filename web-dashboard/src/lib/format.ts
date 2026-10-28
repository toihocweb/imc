export function formatNumber(val: number | string | undefined | null): string {
  if (val === undefined || val === null) return "0";
  const num = typeof val === "string" ? Number(val) : val;
  if (isNaN(num)) return "0";
  return num.toLocaleString("en-US");
}

export function formatBytes(bytes: number | undefined | null): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes.toLocaleString("en-US")} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatPercent(ratio: number | undefined | null): string {
  if (ratio === undefined || ratio === null || isNaN(ratio)) return "0%";
  return `${(ratio * 100).toFixed(1)}%`;
}

export function formatTimestamp(val: number | string | undefined | null): string {
  if (!val) return "—";
  let d: Date;
  if (typeof val === "string") {
    d = new Date(val);
  } else if (val <= 0) {
    return "—";
  } else {
    d = new Date(val * 1000);
  }
  if (isNaN(d.getTime())) return typeof val === "string" ? val : "—";
  const pad = (n: number) => String(n).padStart(2, "0");
  const y = d.getFullYear();
  const m = pad(d.getMonth() + 1);
  const day = pad(d.getDate());
  const h = pad(d.getHours());
  const min = pad(d.getMinutes());
  const s = pad(d.getSeconds());
  return `${y}-${m}-${day} ${h}:${min}:${s}`;
}
