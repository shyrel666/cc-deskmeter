import { LOCALES, MESSAGES, type Lang } from "../i18n/messages";

// Token counts (K/M/B) and USD amounts keep one notation in every language:
// they mirror the API's own units and price list.
const compactUnits: [number, string][] = [
  [1e12, "T"],
  [1e9, "B"],
  [1e6, "M"],
  [1e3, "K"],
];

/** 999 · 12.4K · 3.21M · 1.08B */
export function formatTokens(value: number, digits = 2): string {
  const abs = Math.abs(value);
  for (const [size, unit] of compactUnits) {
    if (abs >= size) {
      const scaled = value / size;
      const places = Math.abs(scaled) >= 100 ? 0 : Math.abs(scaled) >= 10 ? 1 : digits;
      return `${scaled.toFixed(places)}${unit}`;
    }
  }
  return Math.round(value).toString();
}

/** Axis ticks: 0 · 500 · 2K · 1.5M */
export function formatAxisTokens(value: number): string {
  if (value === 0) return "0";
  const abs = Math.abs(value);
  for (const [size, unit] of compactUnits) {
    if (abs >= size) {
      const scaled = value / size;
      return `${Number.isInteger(scaled) ? scaled : scaled.toFixed(1)}${unit}`;
    }
  }
  return Math.round(value).toString();
}

export function formatInteger(value: number): string {
  return Math.round(value).toLocaleString("en-US");
}

const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

/** $1,234.56 · $0.42 · <$0.01 · $12.4K */
export function formatUsd(value: number, { compact = false } = {}): string {
  if (value > 0 && value < 0.01) return "<$0.01";
  if (compact && Math.abs(value) >= 10_000) return `$${formatTokens(value, 1)}`;
  return usd.format(value);
}

/** A plus sign denotes the priced subtotal; unknown prices are never $0. */
export function formatCost(value: number, unpricedCount: number, unknownLabel: string, options = {}): string {
  if (unpricedCount === 0) return formatUsd(value, options);
  return value > 0 ? `${formatUsd(value, options)}+` : unknownLabel;
}

/** 512 B · 12.4 KB · 3.1 MB */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
}

export function formatPercent(ratio: number, digits = 1): string {
  if (!Number.isFinite(ratio)) return "—";
  return `${(ratio * 100).toFixed(digits)}%`;
}

/** Signed change, e.g. +12.4% / −3.0%. */
export function formatDelta(ratio: number): string {
  const sign = ratio > 0 ? "+" : ratio < 0 ? "−" : "±";
  return `${sign}${Math.abs(ratio * 100).toFixed(1)}%`;
}

/** Countdown parts for `Xh Ym` with seconds available separately. */
export function splitDuration(totalSeconds: number) {
  const s = Math.max(0, Math.floor(totalSeconds));
  return {
    hours: Math.floor(s / 3600),
    minutes: Math.floor((s % 3600) / 60),
    seconds: s % 60,
  };
}

export function formatDuration(totalSeconds: number): string {
  const { hours, minutes } = splitDuration(totalSeconds);
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

const pad = (n: number) => n.toString().padStart(2, "0");

export function formatClock(date: Date): string {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function formatRelative(iso: string, now: number, lang: Lang): string {
  const t = MESSAGES[lang].time;
  const diff = Math.max(0, now - new Date(iso).getTime()) / 1000;
  if (diff < 45) return t.justNow;
  if (diff < 3600) return t.minutesAgo(Math.round(diff / 60));
  if (diff < 86_400) return t.hoursAgo(Math.round(diff / 3600));
  if (diff < 86_400 * 14) return t.daysAgo(Math.round(diff / 86_400));
  return new Date(iso).toLocaleDateString(LOCALES[lang], { month: "short", day: "numeric" });
}

/** Parses `YYYY-MM-DD` as a local calendar date. */
export function parseLocalDate(date: string): Date {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function toLocalDateKey(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The backend already writes the home folder as `~`. */
export function displayDataDirs(dirs: string[]): string {
  return dirs.length ? dirs.join(", ") : "~/.claude/projects";
}

const familyNames: Record<string, string> = {
  opus: "Opus",
  sonnet: "Sonnet",
  haiku: "Haiku",
  fable: "Fable",
  mythos: "Fable",
};

/** `claude-opus-4-6-20260101` → `Opus 4.6`; unknown ids are returned as-is. */
export function prettyModel(model: string): string {
  const id = model.toLowerCase().replace(/[._@]/g, "-");
  // Short version numbers only, so `claude-3-5-sonnet-20241022` reaches the
  // legacy pattern instead of reading the date as a major version.
  const match = id.match(/(opus|sonnet|haiku|fable|mythos)-(\d{1,2})(?:-(\d{1,2}))?(?!\d)/);
  if (match) {
    const [, family, major, minor] = match;
    return `${familyNames[family]} ${major}${minor ? `.${minor}` : ""}`;
  }
  const legacy = id.match(/claude-(\d)-(\d)?-?(opus|sonnet|haiku)/);
  if (legacy) {
    const [, major, minor, family] = legacy;
    return `${familyNames[family]} ${major}${minor ? `.${minor}` : ""}`;
  }
  return model;
}
