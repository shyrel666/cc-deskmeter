use std::cmp::Ordering;
use std::ops::AddAssign;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::quota::QuotaStatus;

/// Token counts of one API response, per the CLAUDE.md token formula.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl TokenCounts {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_creation + self.cache_read
    }

    /// Everything sent on the prompt side (uncached + cache write + cache read).
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_creation + self.cache_read
    }

    pub fn saturating_sub(&self, other: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(other.input),
            output: self.output.saturating_sub(other.output),
            cache_creation: self.cache_creation.saturating_sub(other.cache_creation),
            cache_read: self.cache_read.saturating_sub(other.cache_read),
        }
    }
}

impl AddAssign for TokenCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.input += rhs.input;
        self.output += rhs.output;
        self.cache_creation += rhs.cache_creation;
        self.cache_read += rhs.cache_read;
    }
}

/// One assistant response with usage, before cross-file deduplication.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageEntry {
    /// `message.id`, falling back to `requestId`, then to a per-line key.
    /// Shared with every index keyed by it.
    pub dedup_key: Arc<str>,
    /// Stable transcript path and byte position, used to resolve timestamp ties.
    pub source: Arc<str>,
    pub offset: u64,
    /// The response has a non-null `stop_reason`.
    pub complete: bool,
    pub timestamp: DateTime<Utc>,
    /// When the request was sent (at most `timestamp`); 5-hour blocks open on it.
    pub requested_at: DateTime<Utc>,
    pub model: String,
    /// Folder name directly under `~/.claude/projects`.
    pub project: Arc<str>,
    pub session_id: String,
    pub tokens: TokenCounts,
    /// Part of `tokens.cache_creation` written with the 1-hour TTL.
    pub cache_write_1h: u64,
    /// The price tier (see `pricing::tier_cost_usd`), kept so the archive
    /// can sum responses that bill alike.
    pub fast: bool,
    pub long_context: bool,
    pub cost_usd: f64,
    /// Spend as it counts toward the plan quota (see `quota::quota_weight`).
    pub quota_usd: f64,
    /// False when the model has no entry in the pricing table (cost is 0).
    pub priced: bool,
}

impl UsageEntry {
    /// A stable winner across files and reloads. At the same timestamp prefer
    /// a completed response, then a stable source and the later line in it.
    pub fn precedence(&self, other: &Self) -> Ordering {
        self.timestamp
            .cmp(&other.timestamp)
            .then_with(|| self.complete.cmp(&other.complete))
            .then_with(|| self.source.cmp(&other.source))
            .then_with(|| self.offset.cmp(&other.offset))
    }
}

// ---------------------------------------------------------------------------
// Payload sent to the frontend (`get_usage_data` / `token-updated` event).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    /// API-equivalent cost in USD.
    pub cost_usd: f64,
    /// Deduplicated assistant responses.
    pub message_count: u64,
    /// Responses excluded from `cost_usd` because their model is unpriced.
    pub unpriced_message_count: u64,
    /// `cache_read / (input + cache_creation + cache_read)`, in 0.0..=1.0.
    pub cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurnRate {
    pub tokens_per_minute: f64,
    pub cost_per_hour: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    /// Expected block total: usage so far plus the median that recent ended
    /// blocks used after as long into them, or, before any has ended, plus
    /// the burn rate until the block ends.
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveBlock {
    /// The block's first request rounded down to 10 minutes, as Claude does.
    pub start_time: DateTime<Utc>,
    /// `start_time + 5h`.
    pub end_time: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub elapsed_seconds: i64,
    pub remaining_seconds: i64,
    pub usage: UsageStats,
    pub burn_rate: BurnRate,
    pub projected: Projection,
    /// Width of each `timeline` bucket.
    pub bucket_minutes: i64,
    /// Usage per bucket from `start_time` through the bucket holding
    /// `generated_at`, zero-filled; later buckets are omitted.
    pub timeline: Vec<BlockBucket>,
}

/// A 5-hour block that has already ended.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastBlock {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockBucket {
    pub start: DateTime<Utc>,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyUsage {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyUsage {
    /// Local calendar month, `YYYY-MM`.
    pub month: String,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    /// Folder name under `~/.claude/projects` (stable key).
    pub id: String,
    /// Last path segment of the project's working directory.
    pub name: String,
    /// Working directory recorded in the logs, when known.
    pub path: Option<String>,
    pub session_count: usize,
    pub last_activity: DateTime<Utc>,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    /// False when the model is missing from the pricing table (cost counted as 0).
    pub pricing_known: bool,
    #[serde(flatten)]
    pub usage: UsageStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadError {
    pub path: String,
    pub message: String,
}

/// The local archive that keeps usage beyond the transcripts (see `archive`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveStatus {
    /// Folder holding the archive files, `~`-relative.
    pub dir: String,
    /// First archived day, `YYYY-MM-DD`.
    pub since: Option<String>,
    pub days: usize,
    pub blocks: usize,
    /// Weekly-limit readings kept, across accounts.
    pub readings: usize,
    /// Responses counted from the archive alone: their transcripts are gone.
    pub restored_responses: u64,
    /// Why an archive file is not being read or written.
    pub error: Option<String>,
}

/// An ended 5-hour block, live or archived, for the archive view's habits.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRecord {
    pub start_time: DateTime<Utc>,
    /// When the request that opened the block was sent.
    pub opened_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub message_count: u64,
    pub unpriced_message_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFile {
    pub name: String,
    pub bytes: u64,
}

/// What the archive view shows beyond `UsageData`, fetched when it opens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSummary {
    pub generated_at: DateTime<Utc>,
    /// Every local day from the first with usage through today, oldest
    /// first, gaps filled with zeros (snapshots carry the recent ones only).
    pub daily: Vec<DailyUsage>,
    /// Usage counted from the archive alone: its transcripts are gone.
    pub restored: UsageStats,
    /// Every ended block, oldest first.
    pub blocks: Vec<BlockRecord>,
    /// The archive files on disk; empty while no archive is open.
    pub files: Vec<ArchiveFile>,
    /// What "show in folder" reveals; `None` while no archive is open.
    pub reveal_path: Option<String>,
    /// Claude Code's `cleanupPeriodDays`: transcripts older than this are deleted.
    pub retention_days: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageData {
    /// Assigned under the store lock for every snapshot, including idle ticks.
    pub snapshot_id: u64,
    pub generated_at: DateTime<Utc>,
    /// Transcript roots being read (or looked for, while none exists), `~`-relative.
    pub data_dirs: Vec<String>,
    pub file_count: usize,
    /// Prices taken from the local `pricing.json` instead of the built-in table.
    pub custom_prices: usize,
    /// Why `pricing.json` was ignored; the last valid prices stay in use.
    pub pricing_error: Option<String>,
    /// An empty list means the latest reads completed successfully.
    pub read_errors: Vec<ReadError>,
    pub today: UsageStats,
    /// Yesterday up to this time of day: the baseline `today` compares with
    /// (all of yesterday is in `daily`). Responses known only from the
    /// archive have no time of day and are left out.
    pub yesterday_so_far: UsageStats,
    /// The 5-hour block that is still open at `generated_at`, if any.
    pub active_block: Option<ActiveBlock>,
    /// The ended block with the most tokens, the yardstick for the open one.
    pub peak_block: Option<PastBlock>,
    /// One row per local day of the last month or so (from the first day
    /// with usage, if later) through today, oldest first, gaps filled with
    /// zeros. The whole history is in `ArchiveSummary::daily`.
    pub daily: Vec<DailyUsage>,
    /// One row per local month of the last year (from the first month with
    /// usage, if later) through this one, oldest first, gaps filled with zeros.
    pub monthly: Vec<MonthlyUsage>,
    /// The first local day with usage, `YYYY-MM-DD`.
    pub first_day: Option<String>,
    /// Sorted by total tokens, descending.
    pub projects: Vec<ProjectUsage>,
    /// Sorted by cost, descending.
    pub models: Vec<ModelUsage>,
    pub total: UsageStats,
    /// Cache reads' price as a share of the input price, over the models read
    /// from cache (weighted by those reads); `None` without priced reads.
    pub cache_read_price: Option<f64>,
    /// Plan usage from Claude Desktop's local samples plus an estimate since;
    /// `None` when Desktop has not recorded any (or in an unknown format).
    pub quota: Option<QuotaStatus>,
    /// `None` while no archive is open.
    pub archive: Option<ArchiveStatus>,
}
