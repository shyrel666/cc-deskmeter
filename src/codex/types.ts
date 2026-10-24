// Shape of a Codex snapshot, as the backend sends it (`get_codex_data` /
// `codex-updated`, see src-tauri/src/codex/types.rs). Field meanings follow
// Codex's rollout logs (`token_usage_record`, `token_count`, `turn_context`,
// `session_meta`) under CODEX_HOME; costs are what OpenAI's API would charge
// at its Standard tier (src-tauri/src/codex/pricing.rs).

/** Reasoning effort as Codex logs it (`turn_context.effort`), strongest first. */
export const EFFORTS = ["ultra", "max", "xhigh", "high", "medium", "low", "minimal", "none"] as const;
export type Effort = (typeof EFFORTS)[number];

/** Token counts as Codex logs them: `input` includes `cached` (and, taken as such, `cacheWrite`), `output` includes `reasoning`. */
export interface CodexTokens {
  input: number;
  cached: number;
  /** Input written to the prompt cache; zero where the log has no count. */
  cacheWrite: number;
  output: number;
  reasoning: number;
}

/** Tokens and what they would have cost through the OpenAI API. */
export interface CodexUsage {
  tokens: CodexTokens;
  /** API-equivalent cost of the priced responses, USD. */
  costUsd: number;
  /** Responses left out of `costUsd`: their model has no price. */
  unpriced: number;
}

/** What tokens cost through the API, USD, split as `CodexTokens` counts them (so `mixOf` splits it too). */
export type CodexCostSplit = CodexTokens;

/** The cost of each span the overview shows, by kind of token; unpriced responses add nothing. */
export interface CodexSpanCosts {
  today: CodexCostSplit;
  /** The last 7 and 30 local days, today included: the last points of `CodexData.daily`. */
  last7Days: CodexCostSplit;
  last30Days: CodexCostSplit;
  total: CodexCostSplit;
}

/** One account's or model's part of a point: its tokens (`input + output`) and their cost. */
export type CodexSlice = [tokens: number, costUsd: number];

/** One official rate-limit window as of `generatedAt`. */
export interface CodexWindow {
  /** 0–100: the highest reading plus the responses it leaves out; zero once the window reset after the last reading. */
  usedPercent: number;
  windowMinutes: number;
  /** Null once the reading's reset has passed: the next window starts with the next request. */
  resetsAt: string | null;
  /** When Codex logged the reading. */
  observedAt: string;
}

export interface CodexAccount {
  /** Shortened account id: the key the user's name and colour for it are kept under. */
  shortId: string;
  /** Its email, once Codex's sign-in file named it; the default name comes from it. */
  email: string | null;
  plan: string | null;
  /** The window shorter than a day (five hours today); null while the plan's latest limits carry none (Plus from July to August 2026). */
  fiveHour: CodexWindow | null;
  /** The window of a day or longer (a week today), likewise. */
  weekly: CodexWindow | null;
  lastSeen: string | null;
  /** Responses served under the account today. */
  today: CodexUsage;
  /** Responses served under it in its current weekly window (`weekly`, so from `resetsAt − windowMinutes` on): zero once that reset, null without a weekly window. */
  cycle: CodexUsage | null;
  /** Responses served under it in the last 30 days. */
  last30Days: CodexUsage;
  /** Weekly window used, hourly over the last 7 days; the same grid for every account. */
  weekTrend: { t: number; percent: number | null }[];
}

/** The account signed in now, as the latest reading shows it, identified or not. */
export interface CodexCurrent {
  /** Index into `CodexData.accounts`; null while no new thread has said whose these limits are. */
  account: number | null;
  plan: string | null;
  fiveHour: CodexWindow | null;
  weekly: CodexWindow | null;
}

export interface CodexPoint extends CodexUsage {
  key: string;
  /** Start of the hour or local day. */
  start: string;
  current: boolean;
  turns: number;
  /** Per account in `CodexData.accounts` order, then the responses no account could be traced to. */
  byAccount: CodexSlice[];
  /** Per model in `CodexData.seriesModels` order. */
  byModel: CodexSlice[];
}

export interface CodexEffortRow extends CodexUsage {
  effort: string | null;
  turns: number;
}

export interface CodexModelRow extends CodexUsage {
  model: string | null;
  contextWindow: number | null;
  turns: number;
}

/** Context and subagent use over the last 7 days. */
export interface CodexContextStats {
  /** Threads (subagents included) whose context reached 80% of its window. */
  nearFull: number;
  compactions: number;
  subagents: number;
  /** Threads those subagents belong to. */
  spawningThreads: number;
  /** What the subagents spent. */
  subagent: CodexUsage;
  /** What every thread spent, subagents included. */
  total: CodexUsage;
}

/** Turns, tokens and cost with the thread's subagents included. */
export interface CodexThread extends CodexUsage {
  id: string;
  /** `thread_name` from `session_index.jsonl`, else the first prompt, else the agent's nickname. */
  title: string | null;
  path: string | null;
  /** Model and effort of the thread's latest response. */
  model: string | null;
  effort: string | null;
  /** Index into `CodexData.accounts`. */
  account: number | null;
  turns: number;
  /** Largest context of one of the thread's own responses, against the model's window then. */
  contextPeak: number;
  contextWindow: number | null;
  compactions: number;
  subagents: number;
  /** Share of the thread's tokens spent in its subagents. */
  subagentShare: number;
  lastActivity: string;
}

export interface CodexData {
  snapshotId: number;
  generatedAt: string;
  home: string;
  fileCount: number;
  /** Entries of the local `pricing.json` that price a model in these logs instead of the built-in table. */
  customPrices: number;
  /** Why `pricing.json` was ignored; the last valid prices stay in use. */
  pricingError: string | null;
  readErrors: { path: string; message: string }[];
  /** Every account its limits could be traced to, by first reading. */
  accounts: CodexAccount[];
  /** Index of the account signed in now, when identified. */
  activeAccount: number | null;
  current: CodexCurrent | null;
  today: CodexUsage & { turns: number; threads: number; subagents: number };
  hourly: CodexPoint[];
  daily: CodexPoint[];
  /** The models of `CodexPoint.byModel`: every model in `daily`, most tokens first (null: not logged). */
  seriesModels: (string | null)[];
  /** Last 7 days. */
  efforts: CodexEffortRow[];
  models: CodexModelRow[];
  context: CodexContextStats;
  /** Threads active in the last 7 days, subagents folded in. */
  threads: CodexThread[];
  /** Every response read, all time, and every turn they answered. */
  total: CodexUsage & { turns: number };
  spanCosts: CodexSpanCosts;
}

export interface CodexDay extends CodexUsage {
  /** `YYYY-MM-DD`, local. */
  date: string;
  turns: number;
}

/** What the share card shows beyond the snapshot (`get_codex_history`): every day, and all time by model and effort. */
export interface CodexHistory {
  generatedAt: string;
  /** Every local day from the first response's through today, oldest first, zero-filled. */
  daily: CodexDay[];
  /** All time, most tokens first. */
  models: CodexModelRow[];
  /** All time, strongest first. */
  efforts: CodexEffortRow[];
  /** Every response read, all time, and every turn they answered. */
  total: CodexUsage & { turns: number };
}

/** A point of `CodexData.daily` as a day of `CodexHistory.daily`. */
export const dayOf = ({ key, tokens, costUsd, unpriced, turns }: CodexPoint): CodexDay => ({ date: key, tokens, costUsd, unpriced, turns });

export const totalOf = (t: CodexTokens) => t.input + t.output;

export const isEffort = (value: string): value is Effort => (EFFORTS as readonly string[]).includes(value);

/** The four parts of a total that add up to it. */
export const MIX_KEYS = ["reasoning", "output", "fresh", "cached"] as const;
export type MixKey = (typeof MIX_KEYS)[number];

export function mixOf(t: CodexTokens): Record<MixKey, number> {
  return {
    reasoning: t.reasoning,
    output: t.output - t.reasoning,
    fresh: t.input - t.cached,
    cached: t.cached,
  };
}

export function addTokens(a: CodexTokens, b: CodexTokens): CodexTokens {
  return {
    input: a.input + b.input,
    cached: a.cached + b.cached,
    cacheWrite: a.cacheWrite + b.cacheWrite,
    output: a.output + b.output,
    reasoning: a.reasoning + b.reasoning,
  };
}

export const ZERO_TOKENS: CodexTokens = { input: 0, cached: 0, cacheWrite: 0, output: 0, reasoning: 0 };

export function addUsage(a: CodexUsage, b: CodexUsage): CodexUsage {
  return { tokens: addTokens(a.tokens, b.tokens), costUsd: a.costUsd + b.costUsd, unpriced: a.unpriced + b.unpriced };
}

export const ZERO_USAGE: CodexUsage = { tokens: ZERO_TOKENS, costUsd: 0, unpriced: 0 };
