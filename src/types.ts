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

export interface UsageData {
  /** Store-assigned order, including time-only snapshots. */
  snapshotId: number;
  generatedAt: string;
  dataDir: string | null;
  fileCount: number;
  readErrors: { path: string; message: string }[];
  today: UsageStats;
  activeBlock: ActiveBlock | null;
  /** The ended block with the most tokens, the yardstick for the open one. */
  peakBlock: PastBlock | null;
  /** One row per local day, oldest first, through today, zero-filled. */
  daily: DailyUsage[];
  /** Sorted by total tokens, descending. */
  projects: ProjectUsage[];
  /** Sorted by cost, descending. */
  models: ModelUsage[];
  total: UsageStats;
}
