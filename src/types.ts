// Mirrors the Rust payload in `src-tauri/src/usage/types.rs` (serde camelCase).
// Timestamps are RFC 3339 strings in UTC.

export interface UsageStats {
  inputTokens: number;
  outputTokens: number;
  cacheCreationTokens: number;
  cacheReadTokens: number;
  totalTokens: number;
  /** API-equivalent cost in USD. */
  costUsd: number;
  /** Deduplicated assistant responses. */
  messageCount: number;
  /** Responses whose model has no known price; costUsd is a subtotal. */
  unpricedMessageCount: number;
  /** `cacheRead / (input + cacheCreation + cacheRead)`, 0..1. */
  cacheHitRatio: number;
}

export interface BurnRate {
  tokensPerMinute: number;
  costPerHour: number;
}

export interface Projection {
  totalTokens: number;
  costUsd: number;
}

export interface BlockBucket extends UsageStats {
  start: string;
}

/** A 5-hour block that has already ended. */
export interface PastBlock extends UsageStats {
  startTime: string;
  endTime: string;
}

export interface ActiveBlock {
  startTime: string;
  /** `startTime + 5h`. */
  endTime: string;
  lastActivity: string;
  elapsedSeconds: number;
  remainingSeconds: number;
  usage: UsageStats;
  burnRate: BurnRate;
  projected: Projection;
  bucketMinutes: number;
  /** Buckets from the block start through the current one, zero-filled. */
  timeline: BlockBucket[];
}

export interface DailyUsage extends UsageStats {
  /** Local calendar date, `YYYY-MM-DD`. */
  date: string;
}

export interface MonthlyUsage extends UsageStats {
  /** Local calendar month, `YYYY-MM`. */
  month: string;
}

export interface ProjectUsage extends UsageStats {
  id: string;
  name: string;
  path: string | null;
  sessionCount: number;
  lastActivity: string;
}

export interface ModelUsage extends UsageStats {
  model: string;
  pricingKnown: boolean;
}

/** One plan-quota window (5-hour or 7-day), in percent of the limit. */
export interface QuotaWindow {
  /** Latest official value recorded by Claude Desktop. */
  officialPercent: number;
  /** Official value plus estimated use since the sample, 0..100. */
  estimatePercent: number;
  lowPercent: number;
  highPercent: number;
  /** False until enough samples convert spend into %; the estimate then stays at the official value. */
  calibrated: boolean;
  /** The window reset since the sample (per the local 5-hour block); the estimate restarts from 0. */
  resetSinceSample: boolean;
  /** End of the local 5-hour block (5-hour window only). */
  resetsAt: string | null;
  /** Minutes until 100% at the current pace, if before the reset (5-hour window only). */
  minutesToLimit: number | null;
}

export interface QuotaStatus {
  /** When Claude Desktop recorded the latest official value. */
  sampledAt: string;
  /** Desktop stopped sampling: the estimate covers this machine's logs only. */
  stale: boolean;
  /** Null while a window that opened after the latest sample cannot be estimated yet (unknown, not 0). */
  fiveHour: QuotaWindow | null;
  sevenDay: QuotaWindow | null;
  /** The current weekly period; present exactly when `sevenDay` is. */
  week: WeekStatus | null;
}

/** One official 7-day reading. */
export interface WeekPoint {
  at: string;
  percent: number;
}

/** The part of the weekly limit one project or model used on this machine. */
export interface WeekShare {
  id: string;
  /** Project name, or the model id. */
  name: string;
  /** Percentage points of the weekly limit. */
  percent: number;
}

export interface WeekStatus {
  /** Just before the latest drop in the 7-day value (a reset, by Claude or by hand), else the first reading of the last 7 days. */
  periodStart: string;
  /** `periodStart` is an observed drop, so the period opened near 0%. */
  resetObserved: boolean;
  /** Official readings since the drop, at most one per hour, oldest first. */
  points: WeekPoint[];
  /** Where the pace is measured from: a day back, or the period start. */
  paceSince: string | null;
  /** Percentage points per hour; null within the period's first hour. */
  pacePerHour: number | null;
  /** When the estimate reaches 100% at that pace, if within two weeks. */
  limitAt: string | null;
  /** This machine's use since `periodStart`, largest first; empty until calibrated. */
  projects: WeekShare[];
  models: WeekShare[];
  /** Weekly percentage points a full 5-hour window uses up; null until the readings cover one. */
  windowShare: number | null;
}

/** The local archive that keeps usage beyond the transcripts. */
export interface ArchiveStatus {
  /** Folder holding the archive files, `~`-relative. */
  dir: string;
  /** First archived day, `YYYY-MM-DD`. */
  since: string | null;
  days: number;
  blocks: number;
  /** Weekly-limit readings kept, across accounts. */
  readings: number;
  /** Responses counted from the archive alone: their transcripts are gone. */
  restoredResponses: number;
  /** Why an archive file is not being read or written. */
  error: string | null;
}

/** An ended 5-hour block, live or archived. */
export interface BlockRecord {
  startTime: string;
  /** When the request that opened the block was sent. */
  openedAt: string;
  lastActivity: string;
  totalTokens: number;
  costUsd: number;
  messageCount: number;
  unpricedMessageCount: number;
}

export interface ArchiveFile {
  name: string;
  bytes: number;
}

/** What the archive view shows beyond `UsageData`, fetched when it opens. */
export interface ArchiveSummary {
  generatedAt: string;
  /** Every local day from the first with usage through today, oldest first, zero-filled (snapshots carry the recent ones only). */
  daily: DailyUsage[];
  /** Usage counted from the archive alone: its transcripts are gone. */
  restored: UsageStats;
  /** Every ended block, oldest first. */
  blocks: BlockRecord[];
  /** The archive files on disk; empty while no archive is open. */
  files: ArchiveFile[];
  /** What "show in folder" reveals; null while no archive is open. */
  revealPath: string | null;
  /** Claude Code's `cleanupPeriodDays`: transcripts older than this are deleted. */
  retentionDays: number;
}

export interface UsageData {
  /** Store-assigned order, including time-only snapshots. */
  snapshotId: number;
  generatedAt: string;
  /** Transcript roots being read (or looked for, while none exists), `~`-relative. */
  dataDirs: string[];
  fileCount: number;
  /** Prices taken from the local `pricing.json` instead of the built-in table. */
  customPrices: number;
  /** Why `pricing.json` was ignored; the last valid prices stay in use. */
  pricingError: string | null;
  readErrors: { path: string; message: string }[];
  today: UsageStats;
  /** Yesterday up to this time of day, the baseline today compares with (all of yesterday is in `daily`). */
  yesterdaySoFar: UsageStats;
  activeBlock: ActiveBlock | null;
  /** The ended block with the most tokens, the yardstick for the open one. */
  peakBlock: PastBlock | null;
  /** One row per local day of the last month or so (from the first day with usage, if later) through today, oldest first, zero-filled. The whole history is in `ArchiveSummary.daily`. */
  daily: DailyUsage[];
  /** One row per local month of the last year (from the first month with usage, if later) through this one, oldest first, zero-filled. */
  monthly: MonthlyUsage[];
  /** The first local day with usage, `YYYY-MM-DD`. */
  firstDay: string | null;
  /** Sorted by total tokens, descending. */
  projects: ProjectUsage[];
  /** Sorted by cost, descending. */
  models: ModelUsage[];
  total: UsageStats;
  /** Cache reads' price as a share of the input price, over the models read from cache (weighted by those reads); null without priced reads. */
  cacheReadPrice: number | null;
  /** Plan quota from Claude Desktop's local samples plus an estimate since; null without samples. */
  quota: QuotaStatus | null;
  /** Null while no archive is open. */
  archive: ArchiveStatus | null;
}
