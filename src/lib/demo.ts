// Sample payload for running the frontend outside Tauri (`npm run dev` in a
// plain browser). The UI labels it as demo data; it never reaches the app.

import type {
  ActiveBlock,
  BlockBucket,
  DailyUsage,
  ModelUsage,
  PastBlock,
  ProjectUsage,
  UsageData,
  UsageStats,
} from "../types";
import { toLocalDateKey } from "./format";
import { addStats, ZERO_STATS } from "./series";

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
  ["claude-pulse", "D:\\Pycharm Project\\claude-pulse", 0.31],
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

export function makeDemoData(now: Date): UsageData {
  const random = rng(20261017);
  const days = 96;

  // Active block: opened 2h 17m ago, aligned to the minute.
  const blockStart = new Date(now.getTime() - (2 * 3600 + 17 * 60) * 1000);
  blockStart.setSeconds(0, 0);
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
  const activeBlock: ActiveBlock = {
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

  // The record: a long afternoon three days ago.
  const peakStart = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 3, 13, 42);
  const peakBlock: PastBlock = {
    startTime: peakStart.toISOString(),
    endTime: new Date(peakStart.getTime() + 5 * 3600_000).toISOString(),
    ...scale(blockUsage, 1.65),
  };

  // Daily history: busier weekdays, quiet weekends, a ramp-up over time.
  const daily: DailyUsage[] = [];
  let total = ZERO_STATS;
  for (let offset = days - 1; offset >= 0; offset--) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - offset);
    const weekend = date.getDay() === 0 || date.getDay() === 6;
    const ramp = 0.45 + 0.55 * (1 - offset / days);
    let usage = ZERO_STATS;
    if (offset === 0) {
      // Earlier today, before the current block, plus the block itself.
      usage = addStats(scale(burst(random, 0.5), 2.2), blockUsage);
    } else if (random() < (weekend ? 0.35 : 0.9) || offset === 1) {
      // Yesterday always saw work, so today's card has a baseline to compare against.
      const sessions = 1 + Math.floor(random() * (weekend ? 2 : 5) * ramp);
      for (let s = 0; s < sessions; s++) usage = addStats(usage, burst(random, ramp));
    }
    daily.push({ date: toLocalDateKey(date), ...usage });
    total = addStats(total, usage);
  }

  const today = daily[daily.length - 1];

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

  return {
    snapshotId: now.getTime(),
    generatedAt: now.toISOString(),
    readErrors: [],
    dataDir: "~/.claude/projects",
    fileCount: 214,
    today: today,
    activeBlock,
    peakBlock,
    daily,
    projects,
    models,
    total,
  };
}
