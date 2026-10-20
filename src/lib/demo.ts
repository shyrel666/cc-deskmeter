// Sample payload for running the frontend outside Tauri (`npm run dev` in a
// plain browser). The UI labels it as demo data; it never reaches the app.

import type {
  ActiveBlock,
  ArchiveSummary,
  BlockBucket,
  BlockRecord,
  DailyUsage,
  ModelUsage,
  PastBlock,
  ProjectUsage,
  QuotaStatus,
  UsageData,
  UsageStats,
  WeekStatus,
} from "../types";
import { toLocalDateKey } from "./format";
import { addStats, monthlyTotals, ZERO_STATS } from "./series";
import { DAY_MS, HOUR_MS } from "./weekly";

/** Days and months a snapshot carries, as in `src-tauri/src/usage/aggregate.rs`. */
const RECENT_DAYS = 31;
const RECENT_MONTHS = 12;

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Price {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
}

/** List prices per 1M tokens, as per-token rates with 1h cache writes (2x input). */
function perToken(input: number, output: number, cacheRead: number): Price {
  return { input: input / 1e6, output: output / 1e6, cacheWrite: (2 * input) / 1e6, cacheRead: cacheRead / 1e6 };
}

/** The current Claude models: share of the traffic and list prices. */
const MODELS: [model: string, share: number, price: Price][] = [
  ["claude-opus-5-5", 0.5, perToken(4, 20, 0.2)],
  ["claude-sonnet-5-5", 0.24, perToken(2, 10, 0.2)],
  ["claude-fable-5-1", 0.16, perToken(10, 50, 0.25)],
  ["claude-haiku-5-5", 0.1, perToken(0.1, 0.5, 0.01)],
];

/** Totals are billed at the traffic-weighted mix, so the per-model costs add up to them. */
const PRICE: Price = MODELS.reduce(
  (sum, [, share, p]) => ({
    input: sum.input + share * p.input,
    output: sum.output + share * p.output,
    cacheWrite: sum.cacheWrite + share * p.cacheWrite,
    cacheRead: sum.cacheRead + share * p.cacheRead,
  }),
  { input: 0, output: 0, cacheWrite: 0, cacheRead: 0 },
);

function stats(
  input: number,
  output: number,
  cacheWrite: number,
  cacheRead: number,
  messages: number,
  price = PRICE,
): UsageStats {
  const [i, o, w, r] = [input, output, cacheWrite, cacheRead].map(Math.round);
  const prompt = i + w + r;
  return {
    inputTokens: i,
    outputTokens: o,
    cacheCreationTokens: w,
    cacheReadTokens: r,
    totalTokens: i + o + w + r,
    costUsd: i * price.input + o * price.output + w * price.cacheWrite + r * price.cacheRead,
    messageCount: Math.round(messages),
    unpricedMessageCount: 0,
    cacheHitRatio: prompt === 0 ? 0 : r / prompt,
  };
}

function scale(s: UsageStats, factor: number, price = PRICE): UsageStats {
  return stats(
    s.inputTokens * factor,
    s.outputTokens * factor,
    s.cacheCreationTokens * factor,
    s.cacheReadTokens * factor,
    s.messageCount * factor,
    price,
  );
}

/** One Claude Code working session's worth of traffic, `intensity` ~ 0..1. */
function burst(random: () => number, intensity: number): UsageStats {
  const messages = 6 + random() * 90 * intensity;
  return stats(
    messages * (20 + random() * 60),
    messages * (350 + random() * 900),
    messages * (1_500 + random() * 6_000),
    messages * (45_000 + random() * 70_000),
    messages,
  );
}

const PROJECTS: [name: string, path: string, weight: number][] = [
  ["cc-deskmeter", "D:\\Pycharm Project\\cc-deskmeter", 0.31],
  ["sucess_12306", "D:\\Pycharm Project\\sucess_12306", 0.19],
  ["tauri-dashboard", "D:\\work\\tauri-dashboard", 0.14],
  ["rust-notify-lab", "D:\\work\\rust-notify-lab", 0.1],
  ["api-gateway", "D:\\work\\api-gateway", 0.08],
  ["ml-notebooks", "D:\\research\\ml-notebooks", 0.07],
  ["landing-site", "D:\\work\\landing-site", 0.05],
  ["dotfiles", "C:\\Users\\dev\\dotfiles", 0.035],
  ["scratch", "C:\\Users\\dev\\scratch", 0.015],
];

const BUCKET_MINUTES = 10;
/** Claude Code's default `cleanupPeriodDays`: older demo days live on in the archive alone. */
const RETENTION_DAYS = 30;
/** Ended blocks for the archive view: one to four per active day before today,
 *  sharing its usage, none busier than the header's peak block. */
function demoBlocks(daily: DailyUsage[], peakTokens: number): BlockRecord[] {
  const random = rng(5150);
  const blocks: BlockRecord[] = [];
  for (const day of daily.slice(0, -1)) {
    if (day.messageCount === 0) continue;
    const count = Math.min(4, Math.max(1 + Math.floor(random() * 3), Math.ceil(day.totalTokens / (0.95 * peakTokens))));
    // Blocks last five hours, so each opens after the previous one ends; the last by 23:00.
    let hour = Math.min(8 + Math.floor(random() * 3), 23 - 5 * (count - 1));
    let slack = 23 - hour - 5 * (count - 1);
    const [y, m, d] = day.date.split("-").map(Number);
    for (let i = 0; i < count; i++) {
      const opened = new Date(y, m - 1, d, hour, Math.floor(random() * 50) + 3);
      const start = new Date(opened);
      start.setMinutes(0, 0, 0);
      blocks.push({
        startTime: start.toISOString(),
        openedAt: opened.toISOString(),
        lastActivity: new Date(opened.getTime() + (50 + random() * 230) * 60_000).toISOString(),
        totalTokens: Math.round(day.totalTokens / count),
        costUsd: day.costUsd / count,
        messageCount: Math.round(day.messageCount / count),
        unpricedMessageCount: 0,
      });
      const gap = Math.floor(random() * (Math.min(slack, 3) + 1));
      slack -= gap;
      hour += 5 + gap;
    }
  }
  return blocks;
}

/** Days whose transcripts Claude Code would have deleted by `now`. */
function restoredUsage(daily: DailyUsage[], now: Date): UsageStats {
  const cutoff = toLocalDateKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() - RETENTION_DAYS));
  return daily.filter((day) => day.date < cutoff).reduce<UsageStats>((sum, day) => addStats(sum, day), ZERO_STATS);
}

/** Every day of the demo at `now`, which only the archive summary carries, as for the archive view and the share card. */
export function makeDemoHistory(now: Date): DailyUsage[] {
  return demoHistory(now, demoBlock(now));
}

/** The archive view's extras for `data`, which `makeDemoData` built at `now`. */
export function makeDemoArchive(data: UsageData, now: Date): ArchiveSummary {
  const history = makeDemoHistory(now);
  return {
    generatedAt: now.toISOString(),
    daily: history,
    restored: restoredUsage(history, now),
    blocks: demoBlocks(history, data.peakBlock?.totalTokens ?? Infinity),
    files: [
      { name: "archive-daily.json", bytes: 186_420 },
      { name: "archive-blocks.json", bytes: 97_318 },
      { name: "archive-weekly.json", bytes: 241_905 },
    ],
    revealPath: null,
    retentionDays: RETENTION_DAYS,
  };
}

/** Active block: first request 2h 17m before `now`, so the window opened on its 10 minutes. */
function demoBlock(now: Date): ActiveBlock {
  const blockStart = new Date(now.getTime() - (2 * 3600 + 17 * 60) * 1000);
  blockStart.setMinutes(blockStart.getMinutes() - (blockStart.getMinutes() % 10), 0, 0);
  const elapsedSeconds = Math.max(0, Math.floor((now.getTime() - blockStart.getTime()) / 1000));
  const currentBucket = Math.min(29, Math.floor(elapsedSeconds / (BUCKET_MINUTES * 60)));
  const bucketRandom = rng(4242);
  const timeline: BlockBucket[] = [];
  let blockUsage = ZERO_STATS;
  for (let i = 0; i <= currentBucket; i++) {
    // Two bursts of work with a coffee break in between.
    const wave = Math.max(0, Math.sin((i / 30) * Math.PI * 2.4 + 0.3));
    const active = bucketRandom() > 0.18 ? wave * 0.8 + 0.12 : 0;
    let usage = active > 0 ? scale(burst(bucketRandom, active), 0.12) : ZERO_STATS;
    if (i === currentBucket) {
      const into = (elapsedSeconds % (BUCKET_MINUTES * 60)) / (BUCKET_MINUTES * 60);
      usage = scale(usage, Math.max(0.05, into));
    }
    timeline.push({ start: new Date(blockStart.getTime() + i * BUCKET_MINUTES * 60_000).toISOString(), ...usage });
    blockUsage = addStats(blockUsage, usage);
  }
  const elapsedMinutes = Math.max(1, elapsedSeconds / 60);
  const remainingSeconds = Math.max(0, 5 * 3600 - elapsedSeconds);
  const tokensPerMinute = blockUsage.totalTokens / elapsedMinutes;
  const costPerHour = (blockUsage.costUsd / elapsedMinutes) * 60;
  return {
    startTime: blockStart.toISOString(),
    endTime: new Date(blockStart.getTime() + 5 * 3600_000).toISOString(),
    lastActivity: new Date(now.getTime() - 40_000).toISOString(),
    elapsedSeconds,
    remainingSeconds,
    usage: blockUsage,
    burnRate: { tokensPerMinute, costPerHour },
    projected: {
      totalTokens: Math.round(blockUsage.totalTokens + (tokensPerMinute * remainingSeconds) / 60),
      costUsd: blockUsage.costUsd + (costPerHour * remainingSeconds) / 3600,
    },
    bucketMinutes: BUCKET_MINUTES,
    timeline,
  };
}

/** Every day of the demo through today, which ends in `block`: busier weekdays, quiet weekends, a ramp-up over time. */
function demoHistory(now: Date, block: ActiveBlock): DailyUsage[] {
  const random = rng(20261017);
  const days = 96;
  const daily: DailyUsage[] = [];
  for (let offset = days - 1; offset >= 0; offset--) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset);
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    const ramp = 0.45 + 0.55 * (1 - offset / days);
    let usage = ZERO_STATS;
    if (offset === 0) {
      // Earlier today, before the current block, plus the block itself.
      usage = addStats(scale(burst(random, 0.5), 2.2), block.usage);
    } else if (random() < (weekend ? 0.35 : 0.9) || offset === 1) {
      // Yesterday always saw work, so today's card has a baseline to compare against.
      const sessions = 1 + Math.floor(random() * (weekend ? 2 : 5) * ramp);
      for (let s = 0; s < sessions; s++) usage = addStats(usage, burst(random, ramp));
    }
    daily.push({ date: toLocalDateKey(date), ...usage });
  }
  return daily;
}

export function makeDemoData(now: Date): UsageData {
  const activeBlock = demoBlock(now);
  const { remainingSeconds } = activeBlock;

  // The record: a long afternoon three days ago.
  const peakStart = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 3, 13, 0);
  const peakBlock: PastBlock = {
    startTime: peakStart.toISOString(),
    endTime: new Date(peakStart.getTime() + 5 * 3600_000).toISOString(),
    ...scale(activeBlock.usage, 1.65),
  };

  // Snapshots carry the recent days and months; the archive view gets them all.
  const history = demoHistory(now, activeBlock);
  const total = history.reduce<UsageStats>((sum, day) => addStats(sum, day), ZERO_STATS);
  const daily = history.slice(-RECENT_DAYS);
  const today = daily[daily.length - 1];
  // Yesterday's usage spread evenly over its hours, up to this time of day.
  const dayShare = (now.getHours() * 60 + now.getMinutes()) / (24 * 60);
  const yesterdaySoFar = scale(daily[daily.length - 2], dayShare);

  const projects: ProjectUsage[] = PROJECTS.map(([name, path, weight], index) => {
    const usage = scale(total, weight);
    return {
      id: path.replace(/[^A-Za-z0-9]/g, "-"),
      name,
      path,
      sessionCount: Math.max(1, Math.round(140 * weight + (index % 3))),
      lastActivity: new Date(now.getTime() - (index === 0 ? 40_000 : (index * index + 1) * 3_600_000 * 2.7)).toISOString(),
      ...usage,
    };
  });

  const models: ModelUsage[] = MODELS.map(([model, share, price]) => ({
    model,
    pricingKnown: true,
    ...scale(total, share, price),
  }));

  // This week: a reset 2.6 days ago, then Desktop's hourly readings up to the
  // one 7 minutes ago, busy in the daytime.
  const sampledAt = now.getTime() - 7 * 60_000;
  const periodStart = Math.floor((now.getTime() - 2.6 * DAY_MS) / 60_000) * 60_000;
  const readingTimes: number[] = [];
  for (let t = periodStart + 15 * 60_000; t < sampledAt; t += HOUR_MS) readingTimes.push(t);
  readingTimes.push(sampledAt);
  const weekRandom = rng(17);
  let weekUse = 0;
  const cumulative = readingTimes.map((t) => {
    const hour = new Date(t).getHours();
    weekUse += (hour >= 9 && hour < 23 ? 1 : 0.08) * (0.6 + weekRandom());
    return weekUse;
  });
  const weekOfficial = 50;
  const weekEstimate = 51.2;
  const points = readingTimes.map((t, i) => ({
    at: new Date(t).toISOString(),
    percent: Math.round((cumulative[i] / weekUse) * weekOfficial),
  }));
  const dayOld = [...points].reverse().find((p) => now.getTime() - Date.parse(p.at) >= DAY_MS);
  const paceSince = dayOld ? Date.parse(dayOld.at) : periodStart;
  const pacePerHour = (weekEstimate - (dayOld?.percent ?? 0)) / ((now.getTime() - paceSince) / HOUR_MS);
  // About 4.7 points came from the web, phones and other computers.
  const thisComputer = 46.5;
  const week: WeekStatus = {
    periodStart: new Date(periodStart).toISOString(),
    resetObserved: true,
    points,
    paceSince: new Date(paceSince).toISOString(),
    pacePerHour,
    limitAt: new Date(now.getTime() + ((100 - weekEstimate) / pacePerHour) * HOUR_MS).toISOString(),
    projects: PROJECTS.map(([name, path, weight]) => ({ id: path, name, percent: thisComputer * weight })),
    models: MODELS.map(([model, share]) => ({ id: model, name: model, percent: thisComputer * share })),
    windowShare: 13,
  };

  // Desktop's reading from 7 minutes ago, plus what the block used since.
  const toLimit = Math.round((remainingSeconds / 60) * 0.8);
  const quota: QuotaStatus = {
    sampledAt: new Date(sampledAt).toISOString(),
    stale: false,
    fiveHour: {
      officialPercent: 38,
      estimatePercent: 43.6,
      lowPercent: 42,
      highPercent: 46.2,
      calibrated: true,
      resetSinceSample: false,
      resetsAt: activeBlock.endTime,
      minutesToLimit: toLimit > 10 ? toLimit : null,
    },
    sevenDay: {
      officialPercent: weekOfficial,
      estimatePercent: weekEstimate,
      lowPercent: weekOfficial,
      highPercent: 52.3,
      calibrated: true,
      resetSinceSample: false,
      resetsAt: null,
      minutesToLimit: null,
    },
    week,
  };

  return {
    snapshotId: now.getTime(),
    generatedAt: now.toISOString(),
    readErrors: [],
    dataDirs: ["~/.claude/projects"],
    fileCount: 214,
    customPrices: 0,
    pricingError: null,
    today: today,
    yesterdaySoFar,
    // Every model gets its share of the cache reads, so the mix prices them.
    cacheReadPrice: PRICE.cacheRead / PRICE.input,
    activeBlock,
    peakBlock,
    daily,
    monthly: monthlyTotals(history).slice(-RECENT_MONTHS),
    firstDay: history.find((day) => day.messageCount > 0)?.date ?? null,
    projects,
    models,
    total,
    quota,
    archive: {
      dir: "~/AppData/Roaming/com.claudepulse.app",
      since: history[0]?.date ?? null,
      days: history.length,
      blocks: demoBlocks(history, peakBlock.totalTokens).length,
      readings: 2_870,
      restoredResponses: restoredUsage(history, now).messageCount,
      error: null,
    },
  };
}
