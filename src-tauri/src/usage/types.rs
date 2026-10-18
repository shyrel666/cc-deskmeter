use std::cmp::Ordering;
use std::ops::AddAssign;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;

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
    pub cost_usd: f64,
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
    /// Expected block total if the current burn rate holds until the block ends.
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveBlock {
    /// Start of the minute in which the block's first request was sent.
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageData {
    /// Assigned under the store lock for every snapshot, including idle ticks.
    pub snapshot_id: u64,
    pub generated_at: DateTime<Utc>,
    /// Directory being scanned, or `None` if the home directory is unknown.
    pub data_dir: Option<String>,
    pub file_count: usize,
    /// An empty list means the latest reads completed successfully.
    pub read_errors: Vec<ReadError>,
    pub today: UsageStats,
    /// The 5-hour block that is still open at `generated_at`, if any.
    pub active_block: Option<ActiveBlock>,
    /// The ended block with the most tokens, the yardstick for the open one.
    pub peak_block: Option<PastBlock>,
    /// One row per local day, oldest first, gaps filled with zeros.
    pub daily: Vec<DailyUsage>,
    /// Sorted by total tokens, descending.
    pub projects: Vec<ProjectUsage>,
    /// Sorted by cost, descending.
    pub models: Vec<ModelUsage>,
    pub total: UsageStats,
}
