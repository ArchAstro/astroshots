/** Color tokens mirroring the macOS app's palette, expressed for chalk. */
export const theme = {
  brand: "#54e0a0",
  amber: "#f0b86e",
  blue: "#7aa2f7",
  green: "#54e0a0",
  purple: "#b9a8ff",
  red: "#f0727a",
  muted: "#8a8a9a",
  faint: "#5a5a6a",
  text: "#e8e8f2",
  surface: "#1c1b19",
  stage: "#2a2a29",
  selection: "#2e3140",
} as const;

export function relativeTime(fromMs: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.round((nowMs - fromMs) / 1000));
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hr ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days} day${days === 1 ? "" : "s"} ago`;
  const months = Math.round(days / 30);
  if (months < 12) return `${months} mo ago`;
  return `${Math.round(months / 12)} yr ago`;
}

export function clockTime(ms: number): string {
  const date = new Date(ms);
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((value) => String(value).padStart(2, "0"))
    .join(":");
}

export function isoDateTime(ms: number): string {
  return new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");
}

/** `Aug 11, 2026 at 2:54 PM` style, close to Foundation's abbreviated/short formats. */
export function abbreviatedDateTime(input: number | string): string {
  const date = typeof input === "number" ? new Date(input) : new Date(input);
  if (Number.isNaN(date.getTime())) return typeof input === "string" ? input : "";
  return date.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function truncate(text: string, width: number): string {
  if (width <= 0) return "";
  const single = text.replace(/\s+/g, " ").trim();
  if (single.length <= width) return single;
  return width <= 1 ? "…" : `${single.slice(0, width - 1)}…`;
}
