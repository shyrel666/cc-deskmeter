import { LOCALES, MESSAGES, type Lang } from "../i18n/messages";
import type { ActiveBlock, DailyUsage, ModelUsage, UsageStats } from "../types";
import type { ModelFamily, SeriesKey } from "../theme/themes";
import { formatClock, parseLocalDate, toLocalDateKey } from "./format";

export const ZERO_STATS: UsageStats = {
  inputTokens: 0,
  outputTokens: 0,
  cacheCreationTokens: 0,
  cacheReadTokens: 0,
  totalTokens: 0,
  costUsd: 0,
  messageCount: 0,
  unpricedMessageCount: 0,
  cacheHitRatio: 0,
};

export function addStats(a: UsageStats, b: UsageStats): UsageStats {
  const inputTokens = a.inputTokens + b.inputTokens;
  const cacheCreationTokens = a.cacheCreationTokens + b.cacheCreationTokens;
  const cacheReadTokens = a.cacheReadTokens + b.cacheReadTokens;
  const prompt = inputTokens + cacheCreationTokens + cacheReadTokens;
  return {
    inputTokens,
    outputTokens: a.outputTokens + b.outputTokens,
    cacheCreationTokens,
    cacheReadTokens,
    totalTokens: a.totalTokens + b.totalTokens,
    costUsd: a.costUsd + b.costUsd,
    messageCount: a.messageCount + b.messageCount,
    unpricedMessageCount: a.unpricedMessageCount + b.unpricedMessageCount,
    cacheHitRatio: prompt === 0 ? 0 : cacheReadTokens / prompt,
  };
}

/** Real, un-stripped values of one chart point (what the tooltip reports). */
export interface Breakdown extends Record<SeriesKey, number> {
  total: number;
  cost: number;
  messages: number;
  unpricedMessages: number;
}

export function breakdown(stats: UsageStats): Breakdown {
  return {
    output: stats.outputTokens,
    input: stats.inputTokens,
    cacheWrite: stats.cacheCreationTokens,
    cacheRead: stats.cacheReadTokens,
    total: stats.totalTokens,
    cost: stats.costUsd,
    messages: stats.messageCount,
    unpricedMessages: stats.unpricedMessageCount,
  };
}

/**
 * One x position. Series fields hold what is plotted: `cacheRead` is 0 while
 * cache reads are stripped (kept at 0 rather than removed so the bars and
 * areas animate down instead of being re-mounted). `null` marks the future
 * part of the 5h block.
 */
export type ChartPoint = Record<SeriesKey, number | null> & {
  key: string;
  tick: string;
  title: string;
  raw: Breakdown | null;
  current: boolean;
};

function toPoint(
  key: string,
  tick: string,
  title: string,
  stats: UsageStats | null,
  stripCacheRead: boolean,
  current: boolean,
): ChartPoint {
  if (!stats) {
    return { key, tick, title, raw: null, current, output: null, input: null, cacheWrite: null, cacheRead: null };
  }
  const raw = breakdown(stats);
  return {
    key,
    tick,
    title,
    raw,
    current,
    output: raw.output,
    input: raw.input,
    cacheWrite: raw.cacheWrite,
    cacheRead: stripCacheRead ? 0 : raw.cacheRead,
  };
}

export const BLOCK_MINUTES = 5 * 60;

/** Every bucket of the 5h block; buckets after "now" are empty placeholders. */
export function blockPoints(block: ActiveBlock, stripCacheRead: boolean): ChartPoint[] {
  const minutes = block.bucketMinutes > 0 ? block.bucketMinutes : 10;
  const count = Math.ceil(BLOCK_MINUTES / minutes);
  const start = new Date(block.startTime).getTime();
  const last = block.timeline.length - 1;
  return Array.from({ length: count }, (_, i) => {
    const from = new Date(start + i * minutes * 60_000);
    const to = new Date(start + (i + 1) * minutes * 60_000);
    return toPoint(
      `b${i}`,
      formatClock(from),
      `${formatClock(from)} – ${formatClock(to)}`,
      block.timeline[i] ?? null,
      stripCacheRead,
      i === last,
    );
  });
}

/** The last `days` calendar days ending at `todayKey`, zero-filled. */
export function dailyPoints(
  daily: DailyUsage[],
  todayKey: string,
  stripCacheRead: boolean,
  lang: Lang,
  days = 14,
): ChartPoint[] {
  const locale = LOCALES[lang];
  const dayTick = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" });
  const dayTitle = new Intl.DateTimeFormat(locale, { weekday: "short", month: "short", day: "numeric", year: "numeric" });
  const byDate = new Map(daily.map((day) => [day.date, day]));
  const today = parseLocalDate(todayKey);
  return Array.from({ length: days }, (_, i) => {
    const date = new Date(today);
    date.setDate(today.getDate() - (days - 1 - i));
    const key = toLocalDateKey(date);
    return toPoint(key, dayTick.format(date), dayTitle.format(date), byDate.get(key) ?? ZERO_STATS, stripCacheRead, key === todayKey);
  });
}

/** Calendar months up to the current one (at most `months`), zero-filled. */
export function monthlyPoints(
  daily: DailyUsage[],
  todayKey: string,
  stripCacheRead: boolean,
  lang: Lang,
  months = 12,
): ChartPoint[] {
  const locale = LOCALES[lang];
  const monthTick = new Intl.DateTimeFormat(locale, { month: "short" });
  const monthTitle = new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" });
  const byMonth = new Map<string, UsageStats>();
  for (const day of daily) {
    const key = day.date.slice(0, 7);
    byMonth.set(key, addStats(byMonth.get(key) ?? ZERO_STATS, day));
  }
  const today = parseLocalDate(todayKey);
  const first = daily.length > 0 ? parseLocalDate(daily[0].date) : today;
  const span = (today.getFullYear() - first.getFullYear()) * 12 + today.getMonth() - first.getMonth() + 1;
  const count = Math.max(1, Math.min(months, span));

  return Array.from({ length: count }, (_, i) => {
    const date = new Date(today.getFullYear(), today.getMonth() - (count - 1 - i), 1);
    const key = toLocalDateKey(date).slice(0, 7);
    const showYear = date.getMonth() === 0 || i === 0;
    const month = monthTick.format(date);
    const tick = showYear ? MESSAGES[lang].time.monthWithYear(month, String(date.getFullYear()).slice(2)) : month;
    return toPoint(key, tick, monthTitle.format(date), byMonth.get(key) ?? ZERO_STATS, stripCacheRead, i === count - 1);
  });
}

/** Sums the real values over the points that have data. */
export function sumPoints(points: ChartPoint[]): Breakdown {
  const sum: Breakdown = { output: 0, input: 0, cacheWrite: 0, cacheRead: 0, total: 0, cost: 0, messages: 0, unpricedMessages: 0 };
  for (const point of points) {
    if (!point.raw) continue;
    for (const key of Object.keys(sum) as (keyof Breakdown)[]) sum[key] += point.raw[key];
  }
  return sum;
}

export interface DayComparison {
  today: UsageStats;
  yesterday: UsageStats | null;
  /** Relative change of total tokens vs. yesterday; null without a baseline. */
  change: number | null;
}

export function compareWithYesterday(daily: DailyUsage[], today: UsageStats, todayKey: string): DayComparison {
  const date = parseLocalDate(todayKey);
  date.setDate(date.getDate() - 1);
  const yesterdayKey = toLocalDateKey(date);
  const yesterday = daily.find((day) => day.date === yesterdayKey) ?? null;
  const change =
    yesterday && yesterday.totalTokens > 0 ? (today.totalTokens - yesterday.totalTokens) / yesterday.totalTokens : null;
  return { today, yesterday, change };
}

export function modelFamily(model: string): ModelFamily {
  const id = model.toLowerCase();
  if (id.includes("opus")) return "opus";
  if (id.includes("sonnet")) return "sonnet";
  if (id.includes("haiku")) return "haiku";
  if (id.includes("fable") || id.includes("mythos")) return "fable";
  return "other";
}

export const FAMILY_LABEL: Record<ModelFamily, string> = {
  opus: "Opus",
  sonnet: "Sonnet",
  haiku: "Haiku",
  fable: "Fable",
  other: "Other",
};

export interface FamilyShare {
  family: ModelFamily;
  messages: number;
  unpricedMessages: number;
  tokens: number;
  cost: number;
  /** Share of assistant responses (calls), 0..1. */
  share: number;
  models: string[];
}

/** Call share per model family, largest first; `<synthetic>` rows are ignored. */
export function familyShares(models: ModelUsage[]): FamilyShare[] {
  const byFamily = new Map<ModelFamily, FamilyShare>();
  let totalMessages = 0;
  for (const row of models) {
    if (row.model === "<synthetic>") continue;
    const family = modelFamily(row.model);
    const entry = byFamily.get(family) ?? { family, messages: 0, unpricedMessages: 0, tokens: 0, cost: 0, share: 0, models: [] };
    entry.messages += row.messageCount;
    entry.unpricedMessages += row.unpricedMessageCount;
    entry.tokens += row.totalTokens;
    entry.cost += row.costUsd;
    entry.models.push(row.model);
    byFamily.set(family, entry);
    totalMessages += row.messageCount;
  }
  return [...byFamily.values()]
    .map((entry) => ({ ...entry, share: totalMessages === 0 ? 0 : entry.messages / totalMessages }))
    .sort((a, b) => b.messages - a.messages);
}
