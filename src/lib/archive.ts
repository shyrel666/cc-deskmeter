// Lifetime figures for the archive view: what the whole daily history and the
// chain of ended 5-hour blocks say about how Claude Code gets used.

import type { BlockRecord, DailyUsage, UsageStats } from "../types";
import { addStats, ZERO_STATS } from "./series";
import { WEEK_MS } from "./weekly";

export interface Streak {
  days: number;
  /** First and last day, `YYYY-MM-DD`. */
  from: string;
  to: string;
}

export interface MonthUsage {
  /** `YYYY-MM`. */
  key: string;
  usage: UsageStats;
}

export interface LifetimeStats {
  /** Days with at least one response. */
  activeDays: number;
  /** Calendar days from the first active day through today. */
  spanDays: number;
  bestDay: DailyUsage | null;
  longestStreak: Streak | null;
  /** Active days in a row up to today, or up to yesterday while today is still quiet. */
  currentStreak: number;
  /** The latest `monthCount` months of the span, oldest first. */
  months: MonthUsage[];
}

const active = (day: DailyUsage) => day.messageCount > 0;

/** `daily` runs oldest first through today, zero-filled (as the backend sends it). */
export function lifetimeStats(daily: DailyUsage[], todayKey: string, monthCount = 12): LifetimeStats {
  const first = daily.findIndex(active);
  const days = first < 0 ? [] : daily.slice(first).filter((day) => day.date <= todayKey);

  let bestDay = null as DailyUsage | null;
  let longestStreak = null as Streak | null;
  let run = null as Streak | null;
  const months = new Map<string, UsageStats>();
  for (const day of days) {
    const key = day.date.slice(0, 7);
    months.set(key, addStats(months.get(key) ?? ZERO_STATS, day));
    if (!active(day)) {
      run = null;
      continue;
    }
    if (!bestDay || day.totalTokens > bestDay.totalTokens) bestDay = day;
    run = run ? { ...run, days: run.days + 1, to: day.date } : { days: 1, from: day.date, to: day.date };
    if (!longestStreak || run.days > longestStreak.days) longestStreak = run;
  }

  let currentStreak = 0;
  let index = days.length - 1;
  if (index >= 0 && days[index].date === todayKey && !active(days[index])) index--;
  for (; index >= 0 && active(days[index]); index--) currentStreak++;

  return {
    activeDays: days.filter(active).length,
    spanDays: days.length,
    bestDay,
    longestStreak,
    currentStreak,
    months: [...months].slice(-monthCount).map(([key, usage]) => ({ key, usage })),
  };
}

export interface BlockHabits {
  count: number;
  avgTokens: number;
  avgCost: number;
  /** Blocks with an unpriced response, so `avgCost` is a subtotal. */
  unpriced: number;
  /** From the opening request to the last response, in minutes. */
  avgActiveMinutes: number;
  /** Blocks per week over the last four weeks (or since the first block, if later). */
  perWeek: number;
  /** Blocks opened in each local hour of the day. */
  hours: number[];
  /** The hour most blocks open in. */
  peakHour: number | null;
}

const RECENT_WEEKS = 4;

export function blockHabits(blocks: BlockRecord[], now: Date): BlockHabits {
  const hours = Array.from({ length: 24 }, () => 0);
  let tokens = 0;
  let cost = 0;
  let unpriced = 0;
  let activeMs = 0;
  for (const block of blocks) {
    const opened = new Date(block.openedAt);
    hours[opened.getHours()]++;
    tokens += block.totalTokens;
    cost += block.costUsd;
    unpriced += block.unpricedMessageCount > 0 ? 1 : 0;
    activeMs += Math.max(0, Date.parse(block.lastActivity) - opened.getTime());
  }
  const count = blocks.length;

  let perWeek = 0;
  if (count > 0) {
    const sinceFirst = (now.getTime() - Date.parse(blocks[0].openedAt)) / WEEK_MS;
    const weeks = Math.min(RECENT_WEEKS, Math.max(1, sinceFirst));
    const from = now.getTime() - weeks * WEEK_MS;
    perWeek = blocks.filter((block) => Date.parse(block.openedAt) >= from).length / weeks;
  }

  const busiest = Math.max(...hours);
  return {
    count,
    avgTokens: count ? tokens / count : 0,
    avgCost: count ? cost / count : 0,
    unpriced,
    avgActiveMinutes: count ? activeMs / count / 60_000 : 0,
    perWeek,
    hours,
    peakHour: busiest > 0 ? hours.indexOf(busiest) : null,
  };
}
