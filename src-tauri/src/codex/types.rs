//! Payload sent to the frontend (`get_codex_data` / `codex-updated` event).

use std::ops::AddAssign;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::usage::ReadError;

/// Token counts as Codex logs them: `input` includes `cached` (and, taken
/// as such, `cache_write`: see `pricing`), `output` includes `reasoning`, so
/// a total is `input + output`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTokens {
    pub input: u64,
    pub cached: u64,
    /// Input written to the prompt cache; zero where the log has no count.
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
}

impl CodexTokens {
    pub fn total(&self) -> u64 {
        self.input + self.output
    }

    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }

    /// `self - other` field by field, or `None` if any field would go negative.
    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(other.input)?,
            cached: self.cached.checked_sub(other.cached)?,
            cache_write: self.cache_write.checked_sub(other.cache_write)?,
            output: self.output.checked_sub(other.output)?,
            reasoning: self.reasoning.checked_sub(other.reasoning)?,
        })
    }
}

impl AddAssign for CodexTokens {
    fn add_assign(&mut self, rhs: Self) {
        self.input += rhs.input;
        self.cached += rhs.cached;
        self.cache_write += rhs.cache_write;
        self.output += rhs.output;
        self.reasoning += rhs.reasoning;
    }
}

/// Tokens and what they would have cost through the OpenAI API (`pricing`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsage {
    pub tokens: CodexTokens,
    /// API-equivalent cost of the priced responses, USD.
    pub cost_usd: f64,
    /// Responses left out of `cost_usd`: their model has no price.
    pub unpriced: u64,
}

impl CodexUsage {
    /// One response's: `cost` is `None` when its model has no price.
    pub fn of(tokens: CodexTokens, cost: Option<f64>) -> Self {
        Self {
            tokens,
            cost_usd: cost.unwrap_or(0.0),
            unpriced: u64::from(cost.is_none()),
        }
    }
}

impl AddAssign for CodexUsage {
    fn add_assign(&mut self, rhs: Self) {
        self.tokens += rhs.tokens;
        self.cost_usd += rhs.cost_usd;
        self.unpriced += rhs.unpriced;
    }
}

/// What tokens cost through the API, USD, split the way `CodexTokens` counts
/// them: `input` includes `cached` and `cache_write`, `output` includes
/// `reasoning`, so a total is `input + output`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCostSplit {
    pub input: f64,
    pub cached: f64,
    pub cache_write: f64,
    pub output: f64,
    pub reasoning: f64,
}

impl CodexCostSplit {
    pub fn total(&self) -> f64 {
        self.input + self.output
    }
}

impl AddAssign for CodexCostSplit {
    fn add_assign(&mut self, rhs: Self) {
        self.input += rhs.input;
        self.cached += rhs.cached;
        self.cache_write += rhs.cache_write;
        self.output += rhs.output;
        self.reasoning += rhs.reasoning;
    }
}

/// The cost of each span the overview shows, by kind of token: today, the
/// last 7 and 30 local days (today included, so the last points of `daily`)
/// and all time. Unpriced responses add nothing, as to `CodexUsage`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSpanCosts {
    pub today: CodexCostSplit,
    pub last_7_days: CodexCostSplit,
    pub last_30_days: CodexCostSplit,
    pub total: CodexCostSplit,
}

/// One account's or model's part of a point: its tokens (`input + output`)
/// and their cost, sent as `[tokens, costUsd]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct CodexSlice(pub u64, pub f64);

impl AddAssign<CodexUsage> for CodexSlice {
    fn add_assign(&mut self, usage: CodexUsage) {
        self.0 += usage.tokens.total();
        self.1 += usage.cost_usd;
    }
}

/// One official rate-limit window as of `generated_at`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindow {
    /// Share of the window used, 0–100: its highest reading and what the
    /// responses that reading leaves out add (see `accounts`). Zero once the
    /// window reset after the last reading.
    pub used_percent: f64,
    pub window_minutes: u64,
    /// `None` once the reading's reset has passed: the next window starts with
    /// the next request, which no log has shown yet.
    pub resets_at: Option<DateTime<Utc>>,
    /// When Codex logged the reading.
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrendPoint {
    /// Unix milliseconds.
    pub t: i64,
    /// Weekly window used at `t`, 0–100; `None` before the first reading.
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    /// Shortened account id, as rollouts and `auth.json` name it.
    pub short_id: String,
    /// Its email, once `auth.json` (or the copy a switcher set aside) named it.
    pub email: Option<String>,
    pub plan: Option<String>,
    /// The window shorter than a day (five hours today); `None` while the
    /// plan's latest limits carry none (Plus had a weekly one only from July
    /// to August 2026).
    pub five_hour: Option<CodexWindow>,
    /// The window of a day or longer (a week today), likewise.
    pub weekly: Option<CodexWindow>,
    /// Latest response from one of the account's threads.
    pub last_seen: Option<DateTime<Utc>>,
    /// Responses served under the account today.
    pub today: CodexUsage,
    /// Responses served under it in its current weekly window (`weekly`, so
    /// from `resets_at − window_minutes` on), told by the window each
    /// response's reading shows: zero once it reset, `None` without one.
    pub cycle: Option<CodexUsage>,
    /// Responses served under it in the last 30 days.
    pub last_30_days: CodexUsage,
    /// Weekly window used, hourly over the last 7 days, on the same grid for
    /// every account.
    pub week_trend: Vec<TrendPoint>,
}

/// The account signed in now, as the latest reading shows it: its windows
/// even while no new thread has yet said whose they are.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCurrent {
    /// Index into `CodexData::accounts`; `None` while the account is unidentified.
    pub account: Option<usize>,
    pub plan: Option<String>,
    pub five_hour: Option<CodexWindow>,
    pub weekly: Option<CodexWindow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexToday {
    #[serde(flatten)]
    pub usage: CodexUsage,
    /// Turns whose first response was today.
    pub turns: u64,
    /// Threads (with their subagents folded in) that responded today.
    pub threads: u64,
    /// Subagent threads that responded today.
    pub subagents: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTotal {
    #[serde(flatten)]
    pub usage: CodexUsage,
    /// Every turn with a counted response.
    pub turns: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPoint {
    /// `h<hour>` for hours, `YYYY-MM-DD` for days (local time).
    pub key: String,
    pub start: DateTime<Utc>,
    pub current: bool,
    #[serde(flatten)]
    pub usage: CodexUsage,
    pub turns: u64,
    /// Per account in `CodexData::accounts` order, then the responses no
    /// account could be traced to.
    pub by_account: Vec<CodexSlice>,
    /// Per model in `CodexData::series_models` order.
    pub by_model: Vec<CodexSlice>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexEffortRow {
    /// Reasoning effort as Codex logs it (`turn_context.effort`).
    pub effort: Option<String>,
    pub turns: u64,
    #[serde(flatten)]
    pub usage: CodexUsage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelRow {
    pub model: Option<String>,
    /// The latest `model_context_window` logged with the model.
    pub context_window: Option<u64>,
    pub turns: u64,
    #[serde(flatten)]
    pub usage: CodexUsage,
}

/// Context and subagent use over the last 7 days.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexContextStats {
    /// Threads (subagents included) whose context reached 80% of its window.
    pub near_full: u64,
    pub compactions: u64,
    /// Subagent threads that responded.
    pub subagents: u64,
    /// Threads those subagents belong to.
    pub spawning_threads: u64,
    /// What the subagents spent.
    pub subagent: CodexUsage,
    /// What every thread spent, subagents included.
    pub total: CodexUsage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexThread {
    pub id: String,
    /// `thread_name` from `session_index.jsonl`, else the first prompt, else
    /// the agent's nickname.
    pub title: Option<String>,
    /// Working directory, `~`-relative.
    pub path: Option<String>,
    /// Model and effort of the thread's latest response.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Index into `CodexData::accounts`.
    pub account: Option<usize>,
    /// The thread's turns, its subagents' included.
    pub turns: u64,
    /// The thread's usage, its subagents' included.
    #[serde(flatten)]
    pub usage: CodexUsage,
    /// Largest context of any of the thread's own responses (input + output).
    pub context_peak: u64,
    /// The model's window at that response.
    pub context_window: Option<u64>,
    /// Compactions, subagents' included.
    pub compactions: u64,
    pub subagents: u64,
    /// Share of `tokens` spent in subagents, 0–1.
    pub subagent_share: f64,
    pub last_activity: DateTime<Utc>,
}

/// One local day of use.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    #[serde(flatten)]
    pub usage: CodexUsage,
    /// Turns whose first response was that day.
    pub turns: u64,
}

/// What the share card shows beyond the snapshot (`get_codex_history`):
/// every day since the first response, and all time by model and by effort.
/// Rollouts are kept (old ones compressed), so all of it comes from them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHistory {
    pub generated_at: DateTime<Utc>,
    /// Every local day from the first response's through today, zero-filled.
    pub daily: Vec<CodexDay>,
    /// All time, most tokens first.
    pub models: Vec<CodexModelRow>,
    /// All time, by effort order (ultra first).
    pub efforts: Vec<CodexEffortRow>,
    /// Every response read, all time.
    pub total: CodexTotal,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexData {
    /// Assigned under the store lock for every snapshot, including idle ticks.
    pub snapshot_id: u64,
    pub generated_at: DateTime<Utc>,
    /// The Codex home being read, `~`-relative.
    pub home: String,
    /// Rollout files read (one per rollout, whatever copies exist).
    pub file_count: usize,
    /// Entries of the local `pricing.json` that price a model in these logs
    /// instead of the built-in table.
    pub custom_prices: usize,
    /// Why `pricing.json` was ignored; the last valid prices stay in use.
    pub pricing_error: Option<String>,
    /// An empty list means the latest reads completed successfully.
    pub read_errors: Vec<ReadError>,
    /// Every account its limits could be traced to, by first reading.
    pub accounts: Vec<CodexAccount>,
    /// The account signed in now, when identified (`current.account`).
    pub active_account: Option<usize>,
    pub current: Option<CodexCurrent>,
    pub today: CodexToday,
    /// Local hours of today up to the current one.
    pub hourly: Vec<CodexPoint>,
    /// The last 31 local days, today included.
    pub daily: Vec<CodexPoint>,
    /// The models of `CodexPoint::by_model`: every model in `daily`, most
    /// tokens first.
    pub series_models: Vec<Option<String>>,
    /// Last 7 days, by effort order (ultra first).
    pub efforts: Vec<CodexEffortRow>,
    /// Last 7 days, most tokens first.
    pub models: Vec<CodexModelRow>,
    pub context: CodexContextStats,
    /// Threads active in the last 7 days, most tokens first.
    pub threads: Vec<CodexThread>,
    /// Every response read, all time.
    pub total: CodexTotal,
    pub span_costs: CodexSpanCosts,
}
