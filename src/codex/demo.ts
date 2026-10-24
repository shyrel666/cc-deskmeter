// Sample Codex snapshot for the browser preview, and the share card's history
// behind it. The models, effort levels, context windows and plans are the
// ones Codex Desktop logs today, and every cost is what OpenAI's API charges
// for the tokens (Standard tier, as src-tauri/src/codex/pricing.rs prices
// them); the token counts are made up.

import { toLocalDateKey } from "../lib/format";
import { DAY_MS, HOUR_MS } from "../lib/weekly";
import {
  addTokens,
  addUsage,
  dayOf,
  totalOf,
  ZERO_TOKENS,
  ZERO_USAGE,
  type CodexAccount,
  type CodexCostSplit,
  type CodexData,
  type CodexDay,
  type CodexEffortRow,
  type CodexHistory,
  type CodexModelRow,
  type CodexPoint,
  type CodexSlice,
  type CodexThread,
  type CodexTokens,
  type CodexUsage,
  type Effort,
} from "./types";

const limitWindow = (usedPercent: number, windowMinutes: number, resetsAt: number | null, observedAt: number) => ({
  usedPercent,
  windowMinutes,
  resetsAt: resetsAt === null ? null : new Date(resetsAt).toISOString(),
  observedAt: new Date(observedAt).toISOString(),
});

const MINUTE_MS = 60_000;

/** The demo's models, most tokens first (`seriesModels`). */
const MODELS = ["gpt-6-astra", "gpt-6.1-sol"] as const;
type DemoModel = (typeof MODELS)[number];

/** OpenAI's Standard-tier prices, USD per 1M tokens: input, cached input, cache write, output. */
const PRICES: Record<DemoModel, [number, number, number, number]> = {
  "gpt-6-astra": [10, 1, 12.5, 50],
  "gpt-6.1-sol": [2, 0.1, 2.5, 10],
};

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

/** Tokens of `input` prompt tokens: the cached share, output per prompt token and reasoning share of output. */
function tok(input: number, cacheRatio: number, outRatio: number, reasonRatio: number): CodexTokens {
  const output = Math.round(input * outRatio);
  return {
    input: Math.round(input),
    cached: Math.round(input * cacheRatio),
    cacheWrite: 0,
    output,
    reasoning: Math.round(output * reasonRatio),
  };
}

/** What `tokens` of `model` cost by kind of token, as the backend prices responses under 272K input. */
function costSplit(model: DemoModel, tokens: CodexTokens): CodexCostSplit {
  const [input, cached, cacheWrite, output] = PRICES[model];
  const usd = (count: number, rate: number) => (count * rate) / 1e6;
  const uncached = Math.max(0, tokens.input - tokens.cached - tokens.cacheWrite);
  return {
    input: usd(uncached, input) + usd(tokens.cached, cached) + usd(tokens.cacheWrite, cacheWrite),
    cached: usd(tokens.cached, cached),
    cacheWrite: usd(tokens.cacheWrite, cacheWrite),
    output: usd(tokens.output, output),
    reasoning: usd(Math.min(tokens.reasoning, tokens.output), output),
  };
}

/** `tokens` of `model` at its price. */
function priced(model: DemoModel, tokens: CodexTokens): CodexUsage {
  const split = costSplit(model, tokens);
  return { tokens, costUsd: split.input + split.output, unpriced: 0 };
}

/** A share `k` of `usage`, such as a thread's subagents' part. */
function scaled(usage: CodexUsage, k: number): CodexUsage {
  const t = usage.tokens;
  const part = (n: number) => Math.round(n * k);
  return {
    tokens: { input: part(t.input), cached: part(t.cached), cacheWrite: part(t.cacheWrite), output: part(t.output), reasoning: part(t.reasoning) },
    costUsd: usage.costUsd * k,
    unpriced: 0,
  };
}

const sumUsage = (list: CodexUsage[]) => list.reduce(addUsage, ZERO_USAGE);
const sumSplits = (list: CodexCostSplit[]) => list.reduce(addTokens, ZERO_TOKENS);
const sliceOf = (usage: CodexUsage): CodexSlice => [totalOf(usage.tokens), usage.costUsd];
const addSlices = (a: CodexSlice[], b: CodexSlice[]) => a.map(([tokens, cost], i): CodexSlice => [tokens + b[i][0], cost + b[i][1]]);

/** How busy each hour of the day is: mornings, afternoons and late evenings. */
const HOUR_WEIGHT = [0.15, 0.05, 0, 0, 0, 0, 0, 0.1, 0.4, 0.9, 1, 0.8, 0.3, 0.4, 0.9, 1, 1, 0.8, 0.5, 0.6, 0.9, 1, 0.7, 0.3];

const weightAt = (t: number) => HOUR_WEIGHT[new Date(t).getHours()];

/** Weekly readings, hourly: last week's tail climbing to `prevEnd`, a reset, then this week up to `current`. */
function weekTrend(now: number, resetsAt: number, current: number, prevStart: number, prevEnd: number) {
  const from = now - 7 * DAY_MS;
  const weekStart = resetsAt - 7 * DAY_MS;
  const hours: number[] = [];
  for (let t = from; t <= now; t += HOUR_MS) hours.push(t);
  const work = (a: number, b: number) => hours.filter((t) => t >= a && t < b).reduce((sum, t) => sum + weightAt(t), 0);
  const prevWork = work(from, weekStart) || 1;
  const thisWork = work(weekStart, now) || 1;
  let prevDone = 0;
  let thisDone = 0;
  return hours.map((t) => {
    if (t < weekStart) {
      prevDone += weightAt(t);
      return { t, percent: prevStart + ((prevEnd - prevStart) * prevDone) / prevWork };
    }
    thisDone += weightAt(t);
    return { t, percent: (current * thisDone) / thisWork };
  });
}

function startOfDay(t: number) {
  const d = new Date(t);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Days of use before the snapshot's 31, back to when Codex came into use. */
const EARLIER_DAYS = 120;

interface EarlierDay {
  day: CodexDay;
  /** In `MODELS` order. */
  byModel: CodexUsage[];
  split: CodexCostSplit;
}

/**
 * The days before the snapshot's 31, oldest first: light use growing to the
 * pace of the snapshot's first days. Their own seed keeps the snapshot's
 * numbers as they were.
 */
function earlierDays(now: number): EarlierDay[] {
  const random = rng(20261022);
  const between = (lo: number, hi: number) => lo + (hi - lo) * random();
  const today = startOfDay(now);
  const days: EarlierDay[] = [];
  for (let back = 31 + EARLIER_DAYS - 1; back >= 31; back--) {
    const start = startOfDay(today - back * DAY_MS + 12 * HOUR_MS);
    const weekend = [0, 6].includes(new Date(start).getDay());
    const idle = random() < (weekend ? 0.6 : 0.2);
    const ramp = 0.1 + 0.15 * (1 - (back - 31) / EARLIER_DAYS);
    const input = idle ? 0 : 13e6 * (weekend ? 0.4 : 1) * ramp * between(0.4, 1.6);
    const astra = between(0.55, 0.7);
    const tokens = [astra, 1 - astra].map((share) =>
      tok(input * share, between(0.85, 0.93), between(0.008, 0.013), between(0.38, 0.6)),
    );
    const byModel = tokens.map((t, m) => priced(MODELS[m], t));
    days.push({
      day: { date: toLocalDateKey(new Date(start)), ...sumUsage(byModel), turns: Math.round(input / 115_000) },
      byModel,
      split: sumSplits(tokens.map((t, m) => costSplit(MODELS[m], t))),
    });
  }
  return days;
}

export function makeCodexDemo(at: Date): CodexData {
  return buildDemo(at).data;
}

/** The share card's history behind the snapshot `makeCodexDemo(at)` makes. */
export function makeCodexDemoHistory(at: Date): CodexHistory {
  const { data, dayModels, earlier } = buildDemo(at);
  const perDay = [...earlier.map((day) => day.byModel), ...dayModels];
  const input = data.total.tokens.input || 1;
  const models = MODELS.map((model, m): CodexModelRow => {
    const usage = sumUsage(perDay.map((day) => day[m]));
    return {
      model,
      contextWindow: model === "gpt-6.1-sol" ? 475_000 : 258_400,
      turns: Math.round((data.total.turns * usage.tokens.input) / input),
      ...usage,
    };
  }).sort((a, b) => totalOf(b.tokens) - totalOf(a.tokens));
  // The week's mix of efforts, scaled up to all time.
  const week = data.efforts.reduce((sum, row) => sum + totalOf(row.tokens), 0);
  const k = week ? totalOf(data.total.tokens) / week : 0;
  const efforts = data.efforts.map((row): CodexEffortRow => ({ effort: row.effort, turns: Math.round(row.turns * k), ...scaled(row, k) }));
  return { generatedAt: data.generatedAt, daily: [...earlier.map((day) => day.day), ...data.daily.map(dayOf)], models, efforts, total: data.total };
}

function buildDemo(at: Date) {
  const now = at.getTime();
  const random = rng(20261020);
  const between = (lo: number, hi: number) => lo + (hi - lo) * random();

  /**
   * A point of `input` prompt tokens, shared out among the accounts (the
   * last slot: responses no account could be traced to) and the models, so
   * that every split adds up to the point.
   */
  const point = (input: number, accountShares: number[], reasoning: [number, number]) => {
    const astra = between(0.55, 0.7);
    const modelShares = [astra, 1 - astra];
    const byAccount = accountShares.map(() => ZERO_USAGE);
    const byModel = modelShares.map(() => ZERO_USAGE);
    let split = ZERO_TOKENS;
    accountShares.forEach((accountShare, a) => {
      if (accountShare === 0) return;
      modelShares.forEach((modelShare, m) => {
        const tokens = tok(input * accountShare * modelShare, between(0.85, 0.93), between(0.008, 0.013), between(...reasoning));
        const usage = priced(MODELS[m], tokens);
        byAccount[a] = addUsage(byAccount[a], usage);
        byModel[m] = addUsage(byModel[m], usage);
        split = addTokens(split, costSplit(MODELS[m], tokens));
      });
    });
    return { usage: sumUsage(byModel), byAccount, byModel, split };
  };

  // Today, hour by hour: the second account until three hours ago, the third not at all.
  const dayStart = startOfDay(now);
  const hourly: CodexPoint[] = [];
  const hourAccounts: CodexUsage[][] = [];
  const hourModels: CodexUsage[][] = [];
  const hourSplits: CodexCostSplit[] = [];
  for (let h = 0; h <= at.getHours(); h++) {
    const start = dayStart + h * HOUR_MS;
    const input = HOUR_WEIGHT[h] * 1.5e6 * between(0.6, 1.4) * (h === at.getHours() ? at.getMinutes() / 60 + 0.05 : 1);
    const second = start < now - 3 * HOUR_MS ? between(0.2, 0.45) : 0;
    const { usage, byAccount, byModel, split } = point(input, [1 - second, second, 0, 0], [0.4, 0.62]);
    hourAccounts.push(byAccount);
    hourModels.push(byModel);
    hourSplits.push(split);
    hourly.push({
      key: `h${h}`,
      start: new Date(start).toISOString(),
      current: h === at.getHours(),
      ...usage,
      turns: Math.round(input / 115_000),
      byAccount: byAccount.map(sliceOf),
      byModel: byModel.map(sliceOf),
    });
  }
  const todayUsage = sumUsage(hourly);
  const todayTurns = hourly.reduce((sum, p) => sum + p.turns, 0);

  // The last 31 days; today's bar is the hours above. Codex logged no
  // account until 19 days ago, so the days before belong to none.
  const daily: CodexPoint[] = [];
  const dayAccounts: CodexUsage[][] = [];
  const dayModels: CodexUsage[][] = [];
  const daySplits: CodexCostSplit[] = [];
  for (let back = 30; back >= 0; back--) {
    const start = startOfDay(dayStart - back * DAY_MS + 12 * HOUR_MS);
    const day = new Date(start).getDay();
    const key = toLocalDateKey(new Date(start));
    if (back === 0) {
      dayAccounts.push([0, 1, 2, 3].map((a) => sumUsage(hourAccounts.map((accounts) => accounts[a]))));
      dayModels.push(MODELS.map((_, m) => sumUsage(hourModels.map((models) => models[m]))));
      daySplits.push(sumSplits(hourSplits));
      daily.push({
        key,
        start: new Date(start).toISOString(),
        current: true,
        ...todayUsage,
        turns: todayTurns,
        byAccount: hourly.map((p) => p.byAccount).reduce(addSlices),
        byModel: hourly.map((p) => p.byModel).reduce(addSlices),
      });
      continue;
    }
    const weekend = day === 0 || day === 6;
    // Use picked up mid-month: the first days are as light as the weeks before.
    const ramp = back > 24 ? 0.25 : 1;
    const input = 13e6 * (weekend ? 0.4 : 1) * ramp * between(0.5, 1.5);
    const second = between(0.15, 0.35);
    const third = between(0.1, 0.25);
    const shares = back >= 20 ? [0, 0, 0, 1] : [1 - second - third, second, third, 0];
    const { usage, byAccount, byModel, split } = point(input, shares, [0.38, 0.6]);
    dayAccounts.push(byAccount);
    dayModels.push(byModel);
    daySplits.push(split);
    daily.push({
      key,
      start: new Date(start).toISOString(),
      current: false,
      ...usage,
      turns: Math.round(input / 115_000),
      byAccount: byAccount.map(sliceOf),
      byModel: byModel.map(sliceOf),
    });
  }

  const resets = {
    a5h: now + 2 * HOUR_MS + 13 * MINUTE_MS + 20_000,
    aWeek: now + 44 * HOUR_MS + 12 * MINUTE_MS,
    b5h: now + 4 * HOUR_MS + 31 * MINUTE_MS,
    bWeek: now + 122 * HOUR_MS + 5 * MINUTE_MS,
    cWeek: now + 83 * HOUR_MS + 40 * MINUTE_MS,
  };
  /** What account `a` spent today, in the days of its current weekly window and over the last 30 days. */
  const spent = (a: number, weekResetsAt: number) => {
    const cycleStart = startOfDay(weekResetsAt - 7 * DAY_MS);
    return {
      today: sumUsage(hourAccounts.map((accounts) => accounts[a])),
      cycle: sumUsage(dayAccounts.filter((_, d) => Date.parse(daily[d].start) >= cycleStart).map((accounts) => accounts[a])),
      last30Days: sumUsage(dayAccounts.slice(1).map((accounts) => accounts[a])),
    };
  };
  const accounts: CodexAccount[] = [
    {
      shortId: "a3f2…91c0",
      email: "alex.rivera@example.com",
      plan: "plus",
      fiveHour: limitWindow(42, 300, resets.a5h, now - 2 * MINUTE_MS),
      weekly: limitWindow(78, 10080, resets.aWeek, now - 2 * MINUTE_MS),
      lastSeen: new Date(now - 2 * MINUTE_MS).toISOString(),
      ...spent(0, resets.aWeek),
      weekTrend: weekTrend(now, resets.aWeek, 78, 41, 93),
    },
    {
      shortId: "5d7e…0b24",
      email: "sam.chen@example.com",
      plan: "plus",
      fiveHour: limitWindow(9, 300, resets.b5h, now - 3 * HOUR_MS),
      weekly: limitWindow(24, 10080, resets.bWeek, now - 3 * HOUR_MS),
      lastSeen: new Date(now - 3 * HOUR_MS).toISOString(),
      ...spent(1, resets.bWeek),
      weekTrend: weekTrend(now, resets.bWeek, 24, 12, 58),
    },
    {
      // Idle since yesterday: its 5-hour window has reset and no request has opened the next.
      // Not signed in since the app started, so no email names it.
      shortId: "c41b…7e02",
      email: null,
      plan: "pro",
      fiveHour: limitWindow(0, 300, null, now - 26 * HOUR_MS),
      weekly: limitWindow(61, 10080, resets.cWeek, now - 26 * HOUR_MS),
      lastSeen: new Date(now - 26 * HOUR_MS).toISOString(),
      ...spent(2, resets.cWeek),
      weekTrend: weekTrend(now, resets.cWeek, 61, 27, 84),
    },
  ];

  const ago = (ms: number) => new Date(now - ms).toISOString();
  const thread = (
    title: string,
    path: string,
    model: DemoModel,
    effort: Effort,
    account: number,
    turns: number,
    input: number,
    contextPeak: number,
    compactions: number,
    subagents: number,
    subagentShare: number,
    lastActivity: string,
  ): CodexThread => ({
    id: `${title}-${account}`,
    title,
    path,
    model,
    effort,
    account,
    turns,
    ...priced(model, tok(input, 0.89, 0.011, effort === "ultra" ? 0.64 : effort === "max" ? 0.55 : effort === "high" ? 0.41 : 0.22)),
    contextPeak,
    contextWindow: model === "gpt-6.1-sol" ? 475_000 : 258_400,
    compactions,
    subagents,
    subagentShare,
    lastActivity,
  });

  const threads = [
    thread("Wire Codex rollouts into the usage store", "~/code/cc-deskmeter", "gpt-6.1-sol", "ultra", 0, 86, 18.2e6, 431_000, 2, 3, 0.34, ago(2 * MINUTE_MS)),
    thread("Tray panel: second provider row", "~/code/cc-deskmeter", "gpt-6-astra", "max", 0, 41, 7.9e6, 201_000, 0, 0, 0, ago(26 * MINUTE_MS)),
    thread("Audit rate_limits reset drift", "~/code/cc-deskmeter", "gpt-6-astra", "high", 0, 33, 5.1e6, 164_000, 0, 1, 0.12, ago(HOUR_MS + 8 * MINUTE_MS)),
    thread("Migrate billing webhooks to v2", "~/work/billing-api", "gpt-6.1-sol", "ultra", 1, 58, 12.6e6, 388_000, 1, 2, 0.27, ago(3 * HOUR_MS)),
    thread("Flaky e2e: checkout timeout", "~/work/storefront", "gpt-6-astra", "high", 1, 27, 3.8e6, 119_000, 0, 0, 0, ago(5 * HOUR_MS)),
    thread("Review PR: search ranking v3", "~/work/search", "gpt-6-astra", "max", 0, 19, 2.9e6, 143_000, 0, 0, 0, ago(14 * HOUR_MS)),
    thread("Monorepo: split shared ui package", "~/work/storefront", "gpt-6.1-sol", "max", 2, 64, 9.7e6, 455_000, 3, 4, 0.41, ago(DAY_MS + 2 * HOUR_MS)),
    thread("Port chart theme tokens", "~/code/cc-deskmeter", "gpt-6-astra", "medium", 2, 12, 0.9e6, 61_000, 0, 0, 0, ago(DAY_MS + 3 * HOUR_MS)),
    thread("Draft weekly-limit projection", "~/code/cc-deskmeter", "gpt-6.1-sol", "high", 0, 22, 2.6e6, 172_000, 0, 1, 0.08, ago(3 * DAY_MS)),
    thread("Explain Rust notify debounce", "~/notes", "gpt-6-astra", "medium", 1, 6, 0.4e6, 38_000, 0, 0, 0, ago(2 * DAY_MS)),
  ];

  // The last 7 days by effort, each split between the models: input on
  // each, then the cached share, output per input token and reasoning share.
  const byEffort = (
    [
      ["ultra", 410, 18e6, 40e6, 0.91, 0.012, 0.64],
      ["max", 262, 28e6, 8e6, 0.9, 0.011, 0.55],
      ["high", 518, 27e6, 0, 0.88, 0.01, 0.41],
      ["medium", 91, 3.1e6, 0, 0.84, 0.009, 0.22],
    ] as const
  ).map(([effort, turns, astra, sol, cache, out, reason]) => ({
    effort,
    turns,
    byModel: [priced("gpt-6-astra", tok(astra, cache, out, reason)), priced("gpt-6.1-sol", tok(sol, cache, out, reason))],
  }));

  // All time reaches back past the 31 days.
  const earlier = earlierDays(now);
  const allDays = [...earlier.map((day) => day.day), ...daily];
  const data: CodexData = {
    snapshotId: 1,
    generatedAt: at.toISOString(),
    home: "~/.codex",
    fileCount: 128,
    customPrices: 0,
    pricingError: null,
    readErrors: [],
    accounts,
    activeAccount: 0,
    current: { account: 0, plan: accounts[0].plan, fiveHour: accounts[0].fiveHour, weekly: accounts[0].weekly },
    today: { ...todayUsage, turns: todayTurns, threads: 6, subagents: 4 },
    hourly,
    daily,
    seriesModels: [...MODELS],
    efforts: byEffort.map(({ effort, turns, byModel }) => ({ effort, turns, ...sumUsage(byModel) })),
    models: MODELS.map((model, m) => ({
      model,
      contextWindow: model === "gpt-6.1-sol" ? 475_000 : 258_400,
      turns: m === 0 ? 902 : 379,
      ...sumUsage(byEffort.map((row) => row.byModel[m])),
    })),
    context: {
      nearFull: threads.filter((thread) => thread.contextPeak / (thread.contextWindow ?? Infinity) >= 0.8).length,
      compactions: threads.reduce((sum, thread) => sum + thread.compactions, 0),
      subagents: threads.reduce((sum, thread) => sum + thread.subagents, 0),
      spawningThreads: threads.filter((thread) => thread.subagents > 0).length,
      subagent: sumUsage(threads.map((thread) => scaled(thread, thread.subagentShare))),
      total: sumUsage(threads),
    },
    threads,
    total: { ...sumUsage(allDays), turns: allDays.reduce((sum, p) => sum + p.turns, 0) },
    spanCosts: {
      today: sumSplits(hourSplits),
      last7Days: sumSplits(daySplits.slice(-7)),
      last30Days: sumSplits(daySplits.slice(-30)),
      total: sumSplits([...earlier.map((day) => day.split), ...daySplits]),
    },
  };
  return { data, dayModels, earlier };
}
