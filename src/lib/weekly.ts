// Weekly-limit outlook: when the reset comes, what the current pace leads to,
// and the rows of the weekly trend chart. Pure, so it runs under `node --test`.

import type { WeekStatus } from "../types";

export const HOUR_MS = 3_600_000;
export const DAY_MS = 24 * HOUR_MS;
export const WEEK_MS = 7 * DAY_MS;

/** Claude's weekly reset as its usage panel shows it: a local weekday (0 = Sunday) and time. */
export interface ResetTime {
  day: number;
  /** Minutes after local midnight. */
  minute: number;
}

export function isResetTime(value: unknown): value is ResetTime {
  if (typeof value !== "object" || value === null) return false;
  const { day, minute } = value as Record<string, unknown>;
  return (
    Number.isInteger(day) && (day as number) >= 0 && (day as number) <= 6 &&
    Number.isInteger(minute) && (minute as number) >= 0 && (minute as number) < 24 * 60
  );
}

/** The first local `reset` after `now`. */
export function nextReset(reset: ResetTime, now: Date): Date {
  const next = new Date(now);
  next.setHours(Math.floor(reset.minute / 60), reset.minute % 60, 0, 0);
  next.setDate(next.getDate() + ((reset.day - next.getDay() + 7) % 7));
  if (next <= now) next.setDate(next.getDate() + 7);
  return next;
}

export interface ResolvedReset {
  at: Date;
  /** Set by the user, or the observed drop plus whole weeks. */
  source: "manual" | "estimated";
}

/** The next reset: the user's setting, else a week after the observed drop, else unknown. */
export function resolveReset(week: WeekStatus, manual: ResetTime | null, now: Date): ResolvedReset | null {
  if (manual) return { at: nextReset(manual, now), source: "manual" };
  if (!week.resetObserved) return null;
  let at = Date.parse(week.periodStart) + WEEK_MS;
  while (at <= now.getTime()) at += WEEK_MS;
  return { at: new Date(at), source: "estimated" };
}

export interface WeekOutlook {
  /** Percentage points left before the limit. */
  left: number;
  /** When the current pace reaches 100%. */
  limitAt: Date | null;
  /** How long before the reset the limit is reached, in ms. */
  shortBy: number | null;
  /** Use at the reset when the pace does not reach the limit first. */
  atReset: number | null;
  /** Percentage points a day that last exactly until the reset. */
  perDay: number | null;
}

export function weekOutlook(week: WeekStatus, estimate: number, reset: Date | null, now: Date): WeekOutlook {
  const left = Math.max(0, 100 - estimate);
  const limitAt = week.limitAt ? new Date(week.limitAt) : null;
  if (!reset) return { left, limitAt, shortBy: null, atReset: null, perDay: null };
  const untilReset = reset.getTime() - now.getTime();
  const shortBy = limitAt && limitAt < reset ? reset.getTime() - limitAt.getTime() : null;
  const atReset =
    shortBy === null && week.pacePerHour !== null
      ? Math.min(100, estimate + (week.pacePerHour * untilReset) / HOUR_MS)
      : null;
  const perDay = untilReset > 0 ? left / (untilReset / DAY_MS) : null;
  return { left, limitAt, shortBy, atReset, perDay };
}

/** One x position of the weekly chart (`t` in ms). */
export interface WeekRow {
  t: number;
  /** Desktop's readings, then the current estimate. */
  actual: number | null;
  /** From the estimate on at the current pace, until 100% or the reset. */
  projected: number | null;
  /** Even pace from ~0% to 100% at the reset. */
  even: number | null;
}

export interface WeekChart {
  rows: WeekRow[];
  start: number;
  end: number;
}

export function weekChart(week: WeekStatus, estimate: number, reset: Date | null, now: Date): WeekChart {
  const periodStart = Date.parse(week.periodStart);
  const nowMs = now.getTime();
  const resetMs = reset?.getTime() ?? null;
  // Use restarts from ~0% at an observed drop, or at the start of the reset's week.
  const evenFrom =
    resetMs === null ? null : week.resetObserved ? Math.max(periodStart, resetMs - WEEK_MS) : resetMs - WEEK_MS;
  const evenAt = (t: number) =>
    evenFrom === null || resetMs === null || resetMs <= evenFrom
      ? null
      : Math.min(100, Math.max(0, ((t - evenFrom) / (resetMs - evenFrom)) * 100));
  const start = Math.min(periodStart, evenFrom ?? periodStart);
  const end = Math.max(resetMs ?? periodStart + WEEK_MS, nowMs);

  const rows = new Map<number, WeekRow>();
  const put = (t: number, values: Partial<WeekRow>) => {
    const row = rows.get(t) ?? { t, actual: null, projected: null, even: evenAt(t) };
    rows.set(t, { ...row, ...values });
  };
  for (const point of week.points) put(Date.parse(point.at), { actual: point.percent });
  put(nowMs, { actual: estimate, projected: estimate });
  const pace = week.pacePerHour;
  if (pace !== null && pace > 0 && estimate < 100) {
    const limit = nowMs + ((100 - estimate) / pace) * HOUR_MS;
    const until = Math.min(limit, end);
    put(until, { projected: Math.min(100, estimate + (pace * (until - nowMs)) / HOUR_MS) });
  }
  if (evenFrom !== null && resetMs !== null) {
    put(evenFrom, {});
    put(resetMs, {});
  }
  return { rows: [...rows.values()].sort((a, b) => a.t - b.t), start, end };
}

/** Whole days, hours and minutes of a span, for "4d 2h 05m"-style labels. */
export function splitSpan(ms: number): { days: number; hours: number; minutes: number } {
  const minutes = Math.max(0, Math.round(ms / (HOUR_MS / 60)));
  const hours = Math.floor(minutes / 60);
  return { days: Math.floor(hours / 24), hours: hours % 24, minutes: minutes % 60 };
}
