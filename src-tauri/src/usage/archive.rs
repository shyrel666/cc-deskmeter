//! Aggregates-only local archive, so history outlives the transcripts.
//!
//! Claude Code deletes transcripts after `cleanupPeriodDays` (30 by default),
//! and Claude Desktop keeps its plan usage samples for an unknown time. Three
//! JSON files in the app's config folder (next to `tray.json`) keep what the
//! dashboard shows from them:
//!
//! - `archive-daily.json`: per finished local day, the usage per project ×
//!   model × price tier (four token counts, cost, quota weight, responses),
//!   plus the sessions started and the last activity per project;
//! - `archive-blocks.json`: every ended 5-hour block with its request times
//!   and its usage per model × price tier;
//! - `archive-weekly.json`: Desktop's weekly-limit readings, the resets seen
//!   in them and the fitted %-per-$ calibration, per account, each tagged
//!   `official` (Desktop's reading) or `estimate` (derived here).
//!
//! Costs are saved with the price table that produced them and recomputed
//! from the tokens when prices change. Nothing else is kept: no message
//! content, prompts, tool output or raw session ids. Session starts keep
//! SHA-256 tags for deduplication, and an account is only a hash of its
//! organization id. Each file is replaced atomically; deleting
//! one forgets what it held, nothing more.
//!
//! Live data wins: an archived day × project × model, or block, counts only
//! where the transcripts on disk hold fewer of its responses, and then stands
//! in for all of them (see `aggregate::History`), so nothing counts twice.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeDelta, TimeZone, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::aggregate::{EndedBlock, ProjectMeta};
use super::paths::display_dir;
use super::pricing::{self, ModelRates, Pricing};
use super::quota::{self, PercentPerUsd, Sample};
use super::types::{ArchiveFile, ArchiveStatus, TokenCounts, UsageEntry};

const VERSION: u32 = 1;
pub const DAILY_FILE: &str = "archive-daily.json";
pub const BLOCKS_FILE: &str = "archive-blocks.json";
pub const WEEKLY_FILE: &str = "archive-weekly.json";

/// Readings older than this are thinned to one per hour: the estimate only
/// reads the last week, older ones are history.
const FULL_READINGS: TimeDelta = TimeDelta::days(31);
/// How far back the estimate reads (plus the reading just before).
const RECENT_READINGS: TimeDelta = TimeDelta::days(8);
/// An archived calibration stands in for a fit for at most this long.
const CALIBRATION_MAX_AGE: TimeDelta = TimeDelta::days(7);
/// Fits this close to the archived one are the same: a refit on the same
/// readings can still wobble in its last digits, as the spend it divides is
/// summed from a window that starts elsewhere, or the oldest pair (weighing
/// next to nothing) leaves the readings the estimate reads.
const CALIBRATION_TOLERANCE: f64 = 1e-9;

/// Usage of responses that bill alike: one model and price tier (see
/// `pricing::tier_cost_usd`), within a project on a day, or within a block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Folder name under `~/.claude/projects`; absent in a block.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub fast: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub long_context: bool,
    pub responses: u64,
    pub input: u64,
    pub output: u64,
    /// Cache writes of both TTLs, `cache_write_1h` of them with the 1-hour one.
    pub cache_write: u64,
    #[serde(default)]
    pub cache_write_1h: u64,
    pub cache_read: u64,
    /// At the file's `prices`; 0 for a model without a price.
    pub cost_usd: f64,
    /// Spend as it counts toward the plan quota (see `quota::quota_weight`).
    pub quota_usd: f64,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Row {
    fn new(project: &str, entry: &UsageEntry) -> Self {
        Self {
            project: project.to_owned(),
            model: entry.model.clone(),
            fast: entry.fast,
            long_context: entry.long_context,
            ..Self::default()
        }
    }

    pub fn tokens(&self) -> TokenCounts {
        TokenCounts {
            input: self.input,
            output: self.output,
            cache_creation: self.cache_write,
            cache_read: self.cache_read,
        }
    }

    fn add(&mut self, entry: &UsageEntry) {
        self.responses += 1;
        self.input += entry.tokens.input;
        self.output += entry.tokens.output;
        self.cache_write += entry.tokens.cache_creation;
        self.cache_write_1h += entry.cache_write_1h;
        self.cache_read += entry.tokens.cache_read;
        self.cost_usd += entry.cost_usd;
        self.quota_usd += entry.quota_usd;
    }

    /// Recomputes the cost from the tokens; `None` leaves the row unpriced.
    fn reprice(&mut self, rates: Option<&ModelRates>) {
        let tokens = self.tokens();
        let (write_1h, fast, long) = (self.cache_write_1h, self.fast, self.long_context);
        (self.cost_usd, self.quota_usd) = rates.map_or((0.0, 0.0), |rates| {
            (
                pricing::tier_cost_usd(rates, &tokens, write_1h, fast, long),
                quota::quota_weight(rates, &tokens, write_1h, fast, long),
            )
        });
    }
}

/// One finished local day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedDay {
    /// Local calendar date in the zone it was archived in.
    pub date: NaiveDate,
    /// Responses logged in `[start, end)` belong to it, whatever the zone is now.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub projects: Vec<DayProject>,
    #[serde(default)]
    pub usage: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayProject {
    pub project: String,
    /// Sessions whose first response was logged on this day.
    pub sessions: u64,
    /// Stable hashes of sessions started here; older archives only have a count.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub session_tags: BTreeSet<String>,
    pub last_activity: DateTime<Utc>,
}

/// A stable session identity without persisting the raw id or transcript path.
/// Compared only within one project, just like live session ids.
pub fn session_tag(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}

/// One ended 5-hour block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedBlock {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// When the request that opened the block was sent.
    pub first_request: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    #[serde(default)]
    pub usage: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Claude Desktop's own reading.
    Official,
    /// Derived here from the readings and local spend.
    Estimate,
}

/// One of Desktop's plan usage samples, in %.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub at: DateTime<Utc>,
    pub five_hour: f64,
    pub seven_day: f64,
    pub source: Source,
}

impl Reading {
    fn sample(&self) -> Sample {
        Sample {
            at: self.at,
            five_hour: self.five_hour,
            seven_day: self.seven_day,
        }
    }
}

/// A drop in the 7-day value: a new weekly period (see `quota::weekly_reset`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reset {
    /// The readings on either side of it.
    pub before: DateTime<Utc>,
    pub after: DateTime<Utc>,
    pub from_percent: f64,
    pub to_percent: f64,
    /// `official` when the readings are close enough to place it; `estimate`
    /// across a gap (Desktop closed), when it fell somewhere in between.
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Window {
    FiveHour,
    SevenDay,
}

/// A fitted conversion of local spend into the plan's %, the latest of its
/// UTC day per window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calibration {
    /// The latest reading it was fitted on.
    pub at: DateTime<Utc>,
    pub window: Window,
    /// Percentage points per quota-weighted dollar.
    pub percent_per_usd: f64,
    /// The weights of those dollars; a fit made with others is not used.
    #[serde(default)]
    pub weights: Option<quota::Weights>,
    pub source: Source,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// A hash of Desktop's organization id, only to keep accounts apart.
    pub account: String,
    #[serde(default)]
    pub readings: Vec<Reading>,
    #[serde(default)]
    pub resets: Vec<Reset>,
    #[serde(default)]
    pub calibration: Vec<Calibration>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DailyFile {
    version: u32,
    /// Rates the costs were computed with; models without a price are absent.
    #[serde(default)]
    prices: BTreeMap<String, ModelRates>,
    /// Weights the quota spend was computed with; absent before they were kept.
    #[serde(default)]
    quota_weights: Option<quota::Weights>,
    /// Name and folder per project, for when its transcripts are gone.
    #[serde(default)]
    projects: BTreeMap<String, ProjectMeta>,
    #[serde(default)]
    days: Vec<ArchivedDay>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlocksFile {
    version: u32,
    #[serde(default)]
    prices: BTreeMap<String, ModelRates>,
    #[serde(default)]
    quota_weights: Option<quota::Weights>,
    #[serde(default)]
    blocks: Vec<ArchivedBlock>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WeeklyFile {
    version: u32,
    #[serde(default)]
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

/// One archive file and whether it may be written.
#[derive(Debug)]
struct Slot {
    path: PathBuf,
    /// False while the file holds what this build cannot read (a newer
    /// version, or a read error), so it is never overwritten.
    writable: bool,
    dirty: bool,
    error: Option<String>,
}

#[derive(Debug)]
pub struct Archive {
    dir: PathBuf,
    daily: DailyFile,
    blocks: BlocksFile,
    weekly: WeeklyFile,
    daily_slot: Slot,
    blocks_slot: Slot,
    weekly_slot: Slot,
}

impl Archive {
    /// Reads the archive in `dir` and prices it with `pricing`. Missing files
    /// start empty; one this build cannot read is left alone and unused.
    pub fn open(dir: PathBuf, pricing: &Pricing) -> Self {
        let (mut daily, daily_slot) = load::<DailyFile>(dir.join(DAILY_FILE));
        let (mut blocks, blocks_slot) = load::<BlocksFile>(dir.join(BLOCKS_FILE));
        let (mut weekly, weekly_slot) = load::<WeeklyFile>(dir.join(WEEKLY_FILE));
        daily.days.sort_by_key(|day| day.start);
        blocks.blocks.sort_by_key(|block| block.first_request);
        for account in &mut weekly.accounts {
            account.readings.sort_by_key(|reading| reading.at);
        }
        daily.version = VERSION;
        blocks.version = VERSION;
        weekly.version = VERSION;
        let mut archive = Self {
            dir,
            daily,
            blocks,
            weekly,
            daily_slot,
            blocks_slot,
            weekly_slot,
        };
        archive.reprice(pricing);
        archive
    }

    /// Recomputes the costs of models whose price differs from the one they
    /// were computed with, and every quota spend once its weights changed.
    pub fn reprice(&mut self, pricing: &Pricing) {
        let before = std::mem::take(&mut self.daily.prices);
        let reweigh = self.daily.quota_weights.replace(quota::WEIGHTS) != Some(quota::WEIGHTS)
            && !self.daily.days.is_empty();
        reprice(
            self.daily.days.iter_mut().flat_map(|day| &mut day.usage),
            &before,
            pricing,
            reweigh,
        );
        self.daily.prices = price_table(self.daily.days.iter().flat_map(|day| &day.usage), pricing);
        self.daily_slot.dirty |= self.daily.prices != before || reweigh;

        let before = std::mem::take(&mut self.blocks.prices);
        let reweigh = self.blocks.quota_weights.replace(quota::WEIGHTS) != Some(quota::WEIGHTS)
            && !self.blocks.blocks.is_empty();
        reprice(
            self.blocks
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.usage),
            &before,
            pricing,
            reweigh,
        );
        self.blocks.prices = price_table(
            self.blocks.blocks.iter().flat_map(|block| &block.usage),
            pricing,
        );
        self.blocks_slot.dirty |= self.blocks.prices != before || reweigh;
    }

    pub fn days(&self) -> &[ArchivedDay] {
        &self.daily.days
    }

    pub fn blocks(&self) -> &[ArchivedBlock] {
        &self.blocks.blocks
    }

    pub fn projects(&self) -> &BTreeMap<String, ProjectMeta> {
        &self.daily.projects
    }

    /// Replaces the archived days and blocks with `days` and `blocks` (from
    /// `merge_days` and `merge_blocks`, priced with `pricing`) and records the
    /// names of their projects. Returns whether days or blocks changed.
    pub fn update_usage(
        &mut self,
        days: Vec<ArchivedDay>,
        blocks: Vec<ArchivedBlock>,
        meta: &HashMap<Arc<str>, ProjectMeta>,
        pricing: &Pricing,
    ) -> bool {
        let mut changed = false;
        if days != self.daily.days {
            self.daily.days = days;
            self.daily.prices =
                price_table(self.daily.days.iter().flat_map(|day| &day.usage), pricing);
            self.daily.quota_weights = Some(quota::WEIGHTS);
            self.daily_slot.dirty = true;
            changed = true;
        }
        for day in &self.daily.days {
            for project in &day.projects {
                let Some(info) = meta.get(project.project.as_str()) else {
                    continue;
                };
                if self.daily.projects.get(&project.project) != Some(info) {
                    self.daily
                        .projects
                        .insert(project.project.clone(), info.clone());
                    self.daily_slot.dirty = true;
                }
            }
        }
        if blocks != self.blocks.blocks {
            self.blocks.blocks = blocks;
            self.blocks.prices = price_table(
                self.blocks.blocks.iter().flat_map(|block| &block.usage),
                pricing,
            );
            self.blocks.quota_weights = Some(quota::WEIGHTS);
            self.blocks_slot.dirty = true;
            changed = true;
        }
        changed
    }

    /// Adds Desktop's `live` readings of `account` (they win over archived
    /// ones at the same time) and the resets seen in them, and thins
    /// readings older than a month to one per hour.
    pub fn add_readings(&mut self, account: &str, live: &[Sample], now: DateTime<Utc>) {
        if live.is_empty() {
            return;
        }
        let index = match self
            .weekly
            .accounts
            .iter()
            .position(|a| a.account == account)
        {
            Some(index) => index,
            None => {
                self.weekly.accounts.push(Account {
                    account: account.to_owned(),
                    ..Account::default()
                });
                self.weekly.accounts.len() - 1
            }
        };
        let entry = &mut self.weekly.accounts[index];
        let mut by_time: BTreeMap<DateTime<Utc>, Reading> =
            entry.readings.iter().map(|r| (r.at, *r)).collect();
        for sample in live {
            by_time.insert(
                sample.at,
                Reading {
                    at: sample.at,
                    five_hour: sample.five_hour,
                    seven_day: sample.seven_day,
                    source: Source::Official,
                },
            );
        }
        let readings: Vec<Reading> = by_time.into_values().collect();
        // Resets show between consecutive readings; thinned ones could hide
        // or fake one, so only Desktop's and the unthinned readings count.
        let cutoff = now - FULL_READINGS;
        let mut resets = entry.resets.clone();
        for pair in live.windows(2) {
            note_reset(&mut resets, &pair[0], &pair[1]);
        }
        let recent = &readings[readings.partition_point(|r| r.at < cutoff)..];
        for pair in recent.windows(2) {
            note_reset(&mut resets, &pair[0].sample(), &pair[1].sample());
        }
        resets.sort_by_key(|reset| reset.after);
        let readings = thin(readings, cutoff, &resets);
        if readings != entry.readings || resets != entry.resets {
            entry.readings = readings;
            entry.resets = resets;
            self.weekly_slot.dirty = true;
        }
    }

    /// The account with the latest reading, for when Desktop's file is gone.
    pub fn latest_account(&self) -> Option<&str> {
        self.weekly
            .accounts
            .iter()
            .filter_map(|a| Some((a.readings.last()?.at, a.account.as_str())))
            .max()
            .map(|(_, account)| account)
    }

    /// The readings of `account` the estimate needs at `now`: the last eight
    /// days and the one before.
    pub fn recent_samples(&self, account: &str, now: DateTime<Utc>) -> Vec<Sample> {
        let Some(entry) = self.account(account) else {
            return Vec::new();
        };
        let from = entry
            .readings
            .partition_point(|r| r.at < now - RECENT_READINGS)
            .saturating_sub(1);
        entry.readings[from..].iter().map(Reading::sample).collect()
    }

    /// When every account's readings of the last eight days were taken, and
    /// the latest reading before them of any account, which tells who was
    /// signed in as they began: for telling who was signed in when.
    pub fn reading_times(&self, now: DateTime<Utc>) -> Vec<(DateTime<Utc>, &str)> {
        let cutoff = now - RECENT_READINGS;
        let mut times = Vec::new();
        let mut before = None;
        for entry in &self.weekly.accounts {
            let from = entry.readings.partition_point(|r| r.at < cutoff);
            if let Some(reading) = from.checked_sub(1).map(|i| &entry.readings[i]) {
                before = before.max(Some((reading.at, entry.account.as_str())));
            }
            times.extend(
                entry.readings[from..]
                    .iter()
                    .map(|r| (r.at, entry.account.as_str())),
            );
        }
        times.extend(before);
        times
    }

    /// The latest calibration per window, if it is recent enough to stand in
    /// for a fit.
    pub fn calibration(&self, account: &str, now: DateTime<Utc>) -> PercentPerUsd {
        let Some(entry) = self.account(account) else {
            return PercentPerUsd::default();
        };
        let latest = |window| {
            entry
                .calibration
                .iter()
                .rev()
                .find(|c| c.window == window && c.weights == Some(quota::WEIGHTS))
                .filter(|c| now - c.at <= CALIBRATION_MAX_AGE)
                .map(|c| c.percent_per_usd)
        };
        PercentPerUsd {
            five_hour: latest(Window::FiveHour),
            seven_day: latest(Window::SevenDay),
        }
    }

    /// Keeps `fitted`, fitted on readings up to `at`, as the latest fit of
    /// its UTC day.
    pub fn record_calibration(&mut self, account: &str, fitted: PercentPerUsd, at: DateTime<Utc>) {
        let Some(entry) = self
            .weekly
            .accounts
            .iter_mut()
            .find(|a| a.account == account)
        else {
            return;
        };
        for (window, k) in [
            (Window::FiveHour, fitted.five_hour),
            (Window::SevenDay, fitted.seven_day),
        ] {
            let Some(k) = k.filter(|k| k.is_finite()) else {
                continue;
            };
            let record = Calibration {
                at,
                window,
                percent_per_usd: k,
                weights: Some(quota::WEIGHTS),
                source: Source::Estimate,
            };
            match entry.calibration.iter().rposition(|c| c.window == window) {
                Some(i) if entry.calibration[i].at > at => {}
                Some(i)
                    if entry.calibration[i].at == at
                        && entry.calibration[i].weights == record.weights
                        && (entry.calibration[i].percent_per_usd - k).abs()
                            <= CALIBRATION_TOLERANCE * k.abs() => {}
                Some(i) if entry.calibration[i].at.date_naive() == at.date_naive() => {
                    entry.calibration[i] = record;
                    self.weekly_slot.dirty = true;
                }
                _ => {
                    entry.calibration.push(record);
                    self.weekly_slot.dirty = true;
                }
            }
        }
    }

    fn account(&self, account: &str) -> Option<&Account> {
        self.weekly.accounts.iter().find(|a| a.account == account)
    }

    /// An archive that lives in memory only.
    #[cfg(test)]
    pub fn from_parts(days: Vec<ArchivedDay>, blocks: Vec<ArchivedBlock>) -> Self {
        let slot = |name: &str| Slot {
            path: PathBuf::from(name),
            writable: false,
            dirty: false,
            error: None,
        };
        Self {
            dir: PathBuf::new(),
            daily: DailyFile {
                version: VERSION,
                days,
                ..DailyFile::default()
            },
            blocks: BlocksFile {
                version: VERSION,
                blocks,
                ..BlocksFile::default()
            },
            weekly: WeeklyFile::default(),
            daily_slot: slot(DAILY_FILE),
            blocks_slot: slot(BLOCKS_FILE),
            weekly_slot: slot(WEEKLY_FILE),
        }
    }

    /// Writes the files that changed. A failure is kept for the status and
    /// retried on the next call.
    pub fn save(&mut self) {
        store(&mut self.daily_slot, &self.daily);
        store(&mut self.blocks_slot, &self.blocks);
        store(&mut self.weekly_slot, &self.weekly);
    }

    /// The archive files that exist, with their sizes.
    pub fn files(&self) -> Vec<ArchiveFile> {
        [DAILY_FILE, BLOCKS_FILE, WEEKLY_FILE]
            .into_iter()
            .filter_map(|name| {
                let meta = fs::metadata(self.dir.join(name)).ok()?;
                Some(ArchiveFile {
                    name: name.to_owned(),
                    bytes: meta.len(),
                })
            })
            .collect()
    }

    /// The daily file once written, so the folder opens with it selected;
    /// otherwise the folder itself.
    pub fn reveal_path(&self) -> PathBuf {
        let daily = self.dir.join(DAILY_FILE);
        if daily.is_file() {
            daily
        } else {
            self.dir.clone()
        }
    }

    /// `restored` is how many responses only the archive still knows.
    pub fn status(&self, restored: u64) -> ArchiveStatus {
        let errors: Vec<&str> = [&self.daily_slot, &self.blocks_slot, &self.weekly_slot]
            .iter()
            .filter_map(|slot| slot.error.as_deref())
            .collect();
        ArchiveStatus {
            dir: display_dir(&self.dir, dirs::home_dir().as_deref()),
            since: self
                .daily
                .days
                .first()
                .map(|day| day.date.format("%Y-%m-%d").to_string()),
            days: self.daily.days.len(),
            blocks: self.blocks.blocks.len(),
            readings: self.weekly.accounts.iter().map(|a| a.readings.len()).sum(),
            restored_responses: restored,
            error: (!errors.is_empty()).then(|| errors.join("; ")),
        }
    }
}

/// Reads one archive file. A missing file starts empty. One in a newer
/// version, or that cannot be read, is left untouched and never written;
/// one that is not valid JSON is moved aside to `*.corrupt.json`.
fn load<T: DeserializeOwned + Default>(path: PathBuf) -> (T, Slot) {
    let mut slot = Slot {
        path,
        writable: true,
        dirty: false,
        error: None,
    };
    let fail = |slot: &mut Slot, message: String| {
        eprintln!("cc-deskmeter: archive: {message}");
        slot.error = Some(message);
    };
    let text = match fs::read_to_string(&slot.path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (T::default(), slot),
        Err(err) => {
            slot.writable = false;
            let message = format!("{}: {err}", slot.path.display());
            fail(&mut slot, message);
            return (T::default(), slot);
        }
    };
    if let Ok(Header { version }) = serde_json::from_str::<Header>(&text)
        && version != VERSION
    {
        slot.writable = false;
        let message = format!("{}: unsupported version {version}", slot.path.display());
        fail(&mut slot, message);
        return (T::default(), slot);
    }
    match serde_json::from_str::<T>(&text) {
        Ok(value) => (value, slot),
        Err(err) => {
            let aside = slot.path.with_extension("corrupt.json");
            let message = match fs::rename(&slot.path, &aside) {
                Ok(()) => format!(
                    "{} was unreadable ({err}) and was moved to {}",
                    slot.path.display(),
                    aside.display()
                ),
                Err(move_err) => {
                    slot.writable = false;
                    format!(
                        "{}: {err} (could not move it aside: {move_err})",
                        slot.path.display()
                    )
                }
            };
            fail(&mut slot, message);
            (T::default(), slot)
        }
    }
}

fn store<T: Serialize>(slot: &mut Slot, value: &T) {
    if !slot.dirty || !slot.writable {
        return;
    }
    let result = serde_json::to_vec(value)
        .map_err(io::Error::other)
        .and_then(|json| write_atomic(&slot.path, &json));
    match result {
        Ok(()) => {
            slot.dirty = false;
            slot.error = None;
        }
        Err(err) => {
            let message = format!("{}: {err}", slot.path.display());
            // Retried on every snapshot; logged once per distinct error.
            if slot.error.as_ref() != Some(&message) {
                eprintln!("cc-deskmeter: archive: {message}");
            }
            slot.error = Some(message);
        }
    }
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over
/// `path`, so a crash leaves either the old file or the new one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let result = File::create(&tmp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Recomputes the rows whose model is now priced differently than in
/// `before`, or every row when the quota weights changed.
fn reprice<'a>(
    rows: impl Iterator<Item = &'a mut Row>,
    before: &BTreeMap<String, ModelRates>,
    pricing: &Pricing,
    reweigh: bool,
) {
    for row in rows {
        let rates = pricing.rates_for(&row.model);
        if reweigh || rates.as_ref() != before.get(&row.model) {
            row.reprice(rates.as_ref());
        }
    }
}

fn price_table<'a>(
    rows: impl Iterator<Item = &'a Row>,
    pricing: &Pricing,
) -> BTreeMap<String, ModelRates> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut table = BTreeMap::new();
    for row in rows {
        if seen.insert(&row.model)
            && let Some(rates) = pricing.rates_for(&row.model)
        {
            table.insert(row.model.clone(), rates);
        }
    }
    table
}

/// Records a drop from `a` to `b`; of two seen for the same reading after
/// it, the one with the closer reading before it is kept.
fn note_reset(resets: &mut Vec<Reset>, a: &Sample, b: &Sample) {
    let Some(close) = quota::weekly_reset(a, b) else {
        return;
    };
    let reset = Reset {
        before: a.at,
        after: b.at,
        from_percent: a.seven_day,
        to_percent: b.seven_day,
        source: if close {
            Source::Official
        } else {
            Source::Estimate
        },
    };
    match resets.iter_mut().find(|r| r.after == b.at) {
        Some(existing) if existing.before < a.at => *existing = reset,
        Some(_) => {}
        None => resets.push(reset),
    }
}

/// Keeps every reading from `cutoff` on; before it the first of each hour
/// and those on either side of a reset.
fn thin(readings: Vec<Reading>, cutoff: DateTime<Utc>, resets: &[Reset]) -> Vec<Reading> {
    let hour = |at: DateTime<Utc>| at.timestamp().div_euclid(3600);
    let mut previous = None;
    readings
        .into_iter()
        .filter(|reading| {
            let first_of_hour = previous != Some(hour(reading.at));
            previous = Some(hour(reading.at));
            reading.at >= cutoff
                || first_of_hour
                || resets
                    .iter()
                    .any(|reset| reset.before == reading.at || reset.after == reading.at)
        })
        .collect()
}

/// UTC start of local `date`. A zone that skips midnight starts the day at
/// its first local time.
pub fn day_start<Tz: TimeZone>(date: NaiveDate, tz: &Tz) -> DateTime<Utc> {
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    (0..=16)
        .find_map(|quarter| {
            tz.from_local_datetime(&(midnight + TimeDelta::minutes(15 * quarter)))
                .earliest()
        })
        .map_or_else(
            || Utc.from_utc_datetime(&midnight),
            |start| start.with_timezone(&Utc),
        )
}

/// Which day a response counts toward while merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Span {
    /// Index into the archived days.
    Archived(usize),
    /// A day not archived yet, by its start.
    New(DateTime<Utc>),
}

#[derive(Default)]
struct LiveDay<'a> {
    rows: BTreeMap<(&'a str, &'a str, bool, bool), Row>,
    /// Sessions started and last activity per project.
    projects: BTreeMap<&'a str, (BTreeSet<String>, DateTime<Utc>)>,
}

/// The finished local days, merged with the live responses (`entries`, with
/// their local day in `tz`): per day × project × model, the live rows when
/// the transcripts hold at least as many of its responses as the archive,
/// else the archived ones. Live usage outside every archived day fills new
/// days by the local calendar, clipped so that no two days overlap.
pub fn merge_days<'a, Tz: TimeZone>(
    archived: &[ArchivedDay],
    entries: impl IntoIterator<Item = (&'a UsageEntry, NaiveDate)>,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Vec<ArchivedDay> {
    let today = now.with_timezone(tz).date_naive();
    let known_sessions: HashSet<(&str, &str)> = archived
        .iter()
        .flat_map(|day| &day.projects)
        .flat_map(|project| {
            project
                .session_tags
                .iter()
                .map(|tag| (project.project.as_str(), tag.as_str()))
        })
        .collect();
    let mut local_days: HashMap<NaiveDate, (DateTime<Utc>, DateTime<Utc>)> = HashMap::new();
    let mut new_days: HashMap<DateTime<Utc>, (NaiveDate, DateTime<Utc>)> = HashMap::new();
    let mut span_of = |at: DateTime<Utc>, date: NaiveDate| -> Option<Span> {
        let next = archived.partition_point(|day| day.start <= at);
        if next > 0 && at < archived[next - 1].end {
            return Some(Span::Archived(next - 1));
        }
        if date >= today {
            return None;
        }
        let (start, end) = *local_days.entry(date).or_insert_with(|| {
            let end = date
                .succ_opt()
                .map_or(DateTime::<Utc>::MAX_UTC, |next| day_start(next, tz));
            (day_start(date, tz), end)
        });
        let start = next
            .checked_sub(1)
            .map_or(start, |previous| start.max(archived[previous].end));
        let end = archived.get(next).map_or(end, |day| end.min(day.start));
        new_days.insert(start, (date, end));
        Some(Span::New(start))
    };

    let mut live: BTreeMap<Span, LiveDay<'a>> = BTreeMap::new();
    let mut first: HashMap<(&'a str, &'a str), (DateTime<Utc>, NaiveDate)> = HashMap::new();
    for (entry, date) in entries {
        let session = first
            .entry((entry.session_id.as_str(), &*entry.project))
            .or_insert((entry.timestamp, date));
        if entry.timestamp < session.0 {
            *session = (entry.timestamp, date);
        }
        let Some(span) = span_of(entry.timestamp, date) else {
            continue;
        };
        let day = live.entry(span).or_default();
        day.rows
            .entry((
                &*entry.project,
                entry.model.as_str(),
                entry.fast,
                entry.long_context,
            ))
            .or_insert_with(|| Row::new(&entry.project, entry))
            .add(entry);
        let project = day
            .projects
            .entry(&*entry.project)
            .or_insert_with(|| (BTreeSet::new(), entry.timestamp));
        project.1 = project.1.max(entry.timestamp);
    }
    for ((session, project), (at, date)) in first {
        let tag = session_tag(session);
        // Its first live response may have moved to a later day after a
        // partial transcript deletion. Keep the original archived start.
        if known_sessions.contains(&(project, tag.as_str())) {
            continue;
        }
        if let Some(span) = span_of(at, date) {
            live.entry(span)
                .or_default()
                .projects
                .entry(project)
                .or_insert_with(|| (BTreeSet::new(), at))
                .0
                .insert(tag);
        }
    }

    let mut days: Vec<ArchivedDay> = archived
        .iter()
        .enumerate()
        .map(|(index, day)| merge_day(day, live.remove(&Span::Archived(index)).unwrap_or_default()))
        .collect();
    for (span, day) in live {
        let Span::New(start) = span else {
            continue;
        };
        let (date, end) = new_days[&start];
        let empty = ArchivedDay {
            date,
            start,
            end,
            projects: Vec::new(),
            usage: Vec::new(),
        };
        days.push(merge_day(&empty, day));
    }
    days.sort_by_key(|day| day.start);
    reconcile_legacy_sessions(archived, &mut days);
    days
}

/// Tags discovered on a later day may identify an untagged legacy session
/// whose first records are gone. Absorb those anonymous slots rather than
/// persisting both counts. With no original ids left, only a lower bound is
/// knowable: at least the old total, or the union of all tags, whichever is larger.
fn reconcile_legacy_sessions(archived: &[ArchivedDay], days: &mut [ArchivedDay]) {
    let mut before: HashMap<&str, (u64, bool)> = HashMap::new();
    for project in archived.iter().flat_map(|day| &day.projects) {
        let total = before.entry(&project.project).or_default();
        total.0 += project.sessions;
        total.1 |= project.sessions > project.session_tags.len() as u64;
    }
    for (id, (old_total, has_legacy)) in before {
        if !has_legacy {
            continue;
        }
        let projects = days
            .iter()
            .flat_map(|day| &day.projects)
            .filter(|p| p.project == id);
        let total: u64 = projects.clone().map(|p| p.sessions).sum();
        let known: HashSet<&str> = projects
            .flat_map(|p| p.session_tags.iter().map(String::as_str))
            .collect();
        let mut excess = total.saturating_sub(old_total.max(known.len() as u64));
        for project in days
            .iter_mut()
            .flat_map(|day| &mut day.projects)
            .filter(|p| p.project == id)
        {
            let remove = excess.min(
                project
                    .sessions
                    .saturating_sub(project.session_tags.len() as u64),
            );
            project.sessions -= remove;
            excess -= remove;
        }
    }
}

/// The archived and the live rows of one project × model.
type Sides = (Vec<Row>, Vec<Row>);

/// Per project × model, the side holding more responses (live on a tie);
/// Per project, union session tags and keep legacy counts as a lower bound.
/// Old archives cannot identify sessions whose transcripts have already gone.
fn merge_day(archived: &ArchivedDay, live: LiveDay<'_>) -> ArchivedDay {
    let mut cells: BTreeMap<(&str, &str), Sides> = BTreeMap::new();
    for row in &archived.usage {
        cells
            .entry((&row.project, &row.model))
            .or_default()
            .0
            .push(row.clone());
    }
    for ((project, model, _, _), row) in live.rows {
        cells.entry((project, model)).or_default().1.push(row);
    }
    let responses = |rows: &[Row]| rows.iter().map(|row| row.responses).sum::<u64>();
    let usage = cells
        .into_values()
        .flat_map(|(old, new)| {
            if responses(&new) >= responses(&old) {
                new
            } else {
                old
            }
        })
        .collect();
    let mut projects: BTreeMap<String, DayProject> = archived
        .projects
        .iter()
        .map(|p| (p.project.clone(), p.clone()))
        .collect();
    for (project, (session_tags, last)) in live.projects {
        match projects.entry(project.to_owned()) {
            Entry::Vacant(slot) => {
                slot.insert(DayProject {
                    project: project.to_owned(),
                    sessions: session_tags.len() as u64,
                    session_tags,
                    last_activity: last,
                });
            }
            Entry::Occupied(mut slot) => {
                let kept = slot.get_mut();
                kept.session_tags.extend(session_tags);
                kept.sessions = kept.sessions.max(kept.session_tags.len() as u64);
                kept.last_activity = kept.last_activity.max(last);
            }
        }
    }
    ArchivedDay {
        date: archived.date,
        start: archived.start,
        end: archived.end,
        projects: projects.into_values().collect(),
        usage,
    }
}

/// The ended blocks, merged: those the archive still stands in for as they
/// are, the others from their live responses.
pub fn merge_blocks(archived: &[ArchivedBlock], ended: Vec<EndedBlock<'_>>) -> Vec<ArchivedBlock> {
    ended
        .into_iter()
        .filter_map(|block| {
            if block.archived {
                let index = archived
                    .binary_search_by_key(&block.opened, |a| a.first_request)
                    .ok()?;
                return Some(archived[index].clone());
            }
            let mut rows: BTreeMap<(&str, bool, bool), Row> = BTreeMap::new();
            for entry in &block.entries {
                rows.entry((entry.model.as_str(), entry.fast, entry.long_context))
                    .or_insert_with(|| Row::new("", entry))
                    .add(entry);
            }
            Some(ArchivedBlock {
                start: block.start,
                end: block.end,
                first_request: block.opened,
                last_activity: block.last_activity,
                usage: rows.into_values().collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn at(ts: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// One response, its own session; $4 per million input tokens.
    fn entry(key: &str, ts: &str, project: &str, model: &str, input: u64) -> UsageEntry {
        UsageEntry {
            dedup_key: Arc::from(key),
            source: Arc::from("s.jsonl"),
            offset: 0,
            complete: true,
            timestamp: at(ts),
            requested_at: at(ts),
            model: model.to_owned(),
            project: Arc::from(project),
            session_id: format!("session-{key}"),
            tokens: TokenCounts {
                input,
                ..Default::default()
            },
            cache_write_1h: 0,
            fast: false,
            long_context: false,
            cost_usd: input as f64 * 4e-6,
            quota_usd: input as f64 * 4e-6,
            priced: true,
        }
    }

    fn utc_days(entries: &[UsageEntry]) -> impl Iterator<Item = (&UsageEntry, NaiveDate)> {
        entries.iter().map(|e| (e, e.timestamp.date_naive()))
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("cc-deskmeter-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const OPUS: &str = "claude-opus-5-5";

    #[test]
    fn merging_keeps_cells_that_lost_responses_and_takes_live_ones_otherwise() {
        let now = at("2026-10-19T12:00:00Z");
        let full = [
            entry("a1", "2026-10-17T09:00:00Z", "alpha", OPUS, 100),
            entry("a2", "2026-10-17T10:00:00Z", "alpha", OPUS, 200),
            entry("b1", "2026-10-17T11:00:00Z", "beta", OPUS, 50),
            // Today is not finished, so it is not archived.
            entry("t1", "2026-10-19T09:00:00Z", "alpha", OPUS, 7),
        ];
        let archived = merge_days(&[], utc_days(&full), now, &Utc);
        assert_eq!(archived.len(), 1);
        assert_eq!(
            (archived[0].start, archived[0].end),
            (at("2026-10-17T00:00:00Z"), at("2026-10-18T00:00:00Z"))
        );

        // a2's transcript is gone, beta has a response the archive missed,
        // and a day before today has finished since.
        let live = [
            full[0].clone(),
            full[2].clone(),
            entry("b2", "2026-10-17T12:00:00Z", "beta", OPUS, 25),
            entry("c1", "2026-10-18T08:00:00Z", "alpha", OPUS, 3),
            full[3].clone(),
        ];
        let merged = merge_days(&archived, utc_days(&live), now, &Utc);
        let cells = |day: &ArchivedDay| -> Vec<(String, u64, u64)> {
            day.usage
                .iter()
                .map(|r| (r.project.clone(), r.responses, r.input))
                .collect()
        };
        assert_eq!(
            cells(&merged[0]),
            [("alpha".into(), 2, 300), ("beta".into(), 2, 75)]
        );
        assert_eq!(merged[1].date.to_string(), "2026-10-18");
        assert_eq!(cells(&merged[1]), [("alpha".into(), 1, 3)]);
        // Sessions by the day they started, retaining deleted sessions too.
        let sessions: Vec<_> = merged[0]
            .projects
            .iter()
            .map(|p| (p.project.as_str(), p.sessions, p.last_activity))
            .collect();
        assert_eq!(
            sessions,
            [
                ("alpha", 2, at("2026-10-17T10:00:00Z")),
                ("beta", 2, at("2026-10-17T12:00:00Z"))
            ]
        );
        assert_eq!(merge_days(&merged, utc_days(&live), now, &Utc), merged);
    }

    #[test]
    fn days_archived_in_another_zone_never_overlap() {
        let east = FixedOffset::east_opt(8 * 3600).unwrap();
        let west = FixedOffset::west_opt(5 * 3600).unwrap();
        let now = at("2026-10-19T12:00:00Z");
        let local = |entries: &[UsageEntry], tz: &FixedOffset| -> Vec<(UsageEntry, NaiveDate)> {
            entries
                .iter()
                .map(|e| (e.clone(), e.timestamp.with_timezone(tz).date_naive()))
                .collect()
        };
        // At UTC+8, Oct 17 runs from 16:00 UTC the day before.
        let first = local(&[entry("a", "2026-10-17T10:00:00Z", "p", OPUS, 1)], &east);
        let archived = merge_days(&[], first.iter().map(|(e, d)| (e, *d)), now, &east);
        assert_eq!(
            (archived[0].start, archived[0].end),
            (at("2026-10-16T16:00:00Z"), at("2026-10-17T16:00:00Z"))
        );
        // At UTC-5 it runs from 05:00 UTC: the part after the archived day
        // becomes a day of its own, under the same date.
        let more = local(
            &[
                entry("a", "2026-10-17T10:00:00Z", "p", OPUS, 1),
                entry("b", "2026-10-17T06:00:00Z", "p", OPUS, 2),
                entry("c", "2026-10-17T20:00:00Z", "p", OPUS, 4),
            ],
            &west,
        );
        let merged = merge_days(&archived, more.iter().map(|(e, d)| (e, *d)), now, &west);
        let spans: Vec<_> = merged
            .iter()
            .map(|d| (d.date.to_string(), d.start, d.end, d.usage[0].input))
            .collect();
        assert_eq!(
            spans,
            [
                (
                    "2026-10-17".to_owned(),
                    at("2026-10-16T16:00:00Z"),
                    at("2026-10-17T16:00:00Z"),
                    3
                ),
                (
                    "2026-10-17".to_owned(),
                    at("2026-10-17T16:00:00Z"),
                    at("2026-10-18T05:00:00Z"),
                    4
                ),
            ]
        );
    }

    #[test]
    fn files_round_trip_and_reprice_when_prices_change() {
        let tmp = TempDir::new("archive-files");
        let now = at("2026-10-19T12:00:00Z");
        let mut sonnet = entry(
            "b",
            "2026-10-17T11:00:00Z",
            "beta",
            "claude-sonnet-5-5",
            1_000_000,
        );
        // Not what the built-in price gives: shows that the stored cost is kept.
        sonnet.cost_usd = 2.5;
        let entries = [
            entry("a", "2026-10-17T10:00:00Z", "alpha", OPUS, 1_000_000),
            sonnet,
        ];
        let days = merge_days(&[], utc_days(&entries), now, &Utc);
        let pricing = Pricing::default();
        let mut archive = Archive::open(tmp.0.clone(), &pricing);
        assert!(archive.days().is_empty());
        let meta = HashMap::from([(
            Arc::from("alpha"),
            ProjectMeta {
                name: "alpha".into(),
                path: Some("/work/alpha".into()),
            },
        )]);
        assert!(archive.update_usage(days.clone(), Vec::new(), &meta, &pricing));
        archive.save();
        assert!(!tmp.0.join("archive-daily.json.tmp").exists());
        assert!(!tmp.0.join(WEEKLY_FILE).exists(), "nothing to keep yet");
        let read = |name: &str| -> serde_json::Value {
            serde_json::from_str(&fs::read_to_string(tmp.0.join(name)).unwrap()).unwrap()
        };
        let json = read(DAILY_FILE);
        assert_eq!(json["version"], 1);
        assert_eq!(json["prices"][OPUS]["input"], 4.0);
        assert_eq!(json["projects"]["alpha"]["path"], "/work/alpha");
        assert_eq!(
            json["days"][0]["projects"][0]["sessionTags"][0],
            session_tag("session-a")
        );
        let text = fs::read_to_string(tmp.0.join(DAILY_FILE)).unwrap();
        assert!(!text.contains("session-a") && !text.contains("session-b"));

        let cost = |archive: &Archive, model: &str| {
            archive.days()[0]
                .usage
                .iter()
                .find(|row| row.model == model)
                .unwrap()
                .cost_usd
        };
        assert_eq!(Archive::open(tmp.0.clone(), &pricing).days(), &days[..]);
        // A local price for Opus reprices its rows from their tokens.
        let custom =
            Pricing::parse(r#"{ "claude-opus-5-5": { "input": 9, "output": 45 } }"#).unwrap();
        let mut repriced = Archive::open(tmp.0.clone(), &custom);
        assert!((cost(&repriced, OPUS) - 9.0).abs() < 1e-9);
        assert_eq!(cost(&repriced, "claude-sonnet-5-5"), 2.5);
        repriced.save();
        assert_eq!(read(DAILY_FILE)["prices"][OPUS]["input"], 9.0);
        repriced.reprice(&pricing);
        assert!((cost(&repriced, OPUS) - 4.0).abs() < 1e-9);

        // Quota spend saved with other weights is recomputed from the tokens,
        // even where the price is unchanged.
        let quota = |archive: &Archive, model: &str| {
            archive.days()[0]
                .usage
                .iter()
                .find(|row| row.model == model)
                .unwrap()
                .quota_usd
        };
        assert_eq!(
            read(DAILY_FILE)["quotaWeights"]["cacheWrite"],
            quota::WEIGHTS.cache_write
        );
        assert_eq!(quota(&repriced, "claude-sonnet-5-5"), 4.0, "kept as saved");
        let mut legacy = read(DAILY_FILE);
        legacy.as_object_mut().unwrap().remove("quotaWeights");
        fs::write(tmp.0.join(DAILY_FILE), legacy.to_string()).unwrap();
        let mut reweighed = Archive::open(tmp.0.clone(), &custom);
        assert!((quota(&reweighed, "claude-sonnet-5-5") - 2.0).abs() < 1e-9);
        assert!((quota(&reweighed, OPUS) - 9.0).abs() < 1e-9);
        reweighed.save();
        assert_eq!(
            read(DAILY_FILE)["quotaWeights"]["cacheRead"],
            quota::WEIGHTS.cache_read
        );
    }

    #[test]
    fn legacy_session_counts_migrate_without_counting_partial_sessions_twice() {
        use super::super::aggregate::History;

        let tmp = TempDir::new("archive-legacy-sessions");
        let now = at("2026-10-20T12:00:00Z");
        let early = entry("a", "2026-10-17T10:00:00Z", "p", OPUS, 100);
        let mut later = entry("b", "2026-10-18T10:00:00Z", "p", OPUS, 200);
        later.session_id = early.session_id.clone();
        let entries = [early, later];
        let pricing = Pricing::default();
        let days = merge_days(&[], utc_days(&entries), now, &Utc);
        let mut original = Archive::open(tmp.0.clone(), &pricing);
        original.update_usage(days, Vec::new(), &HashMap::new(), &pricing);
        original.save();

        // Version 1 originally had only counts; the new field is optional.
        let path = tmp.0.join(DAILY_FILE);
        let mut json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for day in json["days"].as_array_mut().unwrap() {
            for project in day["projects"].as_array_mut().unwrap() {
                project.as_object_mut().unwrap().remove("sessionTags");
            }
        }
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let mut legacy = Archive::open(tmp.0.clone(), &pricing);
        assert!(legacy.status(0).error.is_none());
        assert!(
            legacy
                .days()
                .iter()
                .flat_map(|d| &d.projects)
                .all(|p| p.session_tags.is_empty())
        );

        for retained in [&entries[..], &entries[1..], &entries[2..]] {
            let mut history = History::default();
            history.set_archive(&legacy, &pricing);
            for entry in retained {
                history.insert(Arc::new(entry.clone()), &Utc);
            }
            let data = history.snapshot(&HashMap::new(), now, &Utc);
            assert_eq!(data.projects[0].session_count, 1);
            assert_eq!(data.total.input_tokens, 300);
            let merged = merge_days(legacy.days(), utc_days(retained), now, &Utc);
            assert_eq!(
                merged
                    .iter()
                    .flat_map(|d| &d.projects)
                    .map(|p| p.sessions)
                    .sum::<u64>(),
                1
            );
            assert_eq!(merge_days(&merged, utc_days(retained), now, &Utc), merged);
        }

        // Once a surviving session acquires a tag, later launches can match
        // it exactly, even though its original first response is gone.
        let migrated = merge_days(legacy.days(), utc_days(&entries[1..]), now, &Utc);
        assert!(legacy.update_usage(migrated.clone(), Vec::new(), &HashMap::new(), &pricing));
        legacy.save();
        let reopened = Archive::open(tmp.0.clone(), &pricing);
        assert_eq!(reopened.days(), migrated);
        let mut history = History::default();
        history.set_archive(&reopened, &pricing);
        history.insert(Arc::new(entries[1].clone()), &Utc);
        history.insert(
            Arc::new(entry("new", "2026-10-20T10:00:00Z", "p", OPUS, 50)),
            &Utc,
        );
        let data = history.snapshot(&HashMap::new(), now, &Utc);
        assert_eq!(data.projects[0].session_count, 2);
        assert_eq!(data.total.input_tokens, 350);
    }

    #[test]
    fn unreadable_files_are_set_aside_and_newer_ones_left_alone() {
        let tmp = TempDir::new("archive-unreadable");
        let newer = r#"{"version":99,"blocks":"some future format"}"#;
        fs::write(tmp.0.join(DAILY_FILE), "{ not json").unwrap();
        fs::write(tmp.0.join(BLOCKS_FILE), newer).unwrap();
        let pricing = Pricing::default();
        let mut archive = Archive::open(tmp.0.clone(), &pricing);
        assert_eq!(
            fs::read_to_string(tmp.0.join("archive-daily.corrupt.json")).unwrap(),
            "{ not json"
        );
        let error = archive.status(0).error.unwrap();
        assert!(
            error.contains("moved") && error.contains("unsupported version 99"),
            "{error}"
        );

        let entries = [entry("a", "2026-10-17T10:00:00Z", "p", OPUS, 1)];
        let days = merge_days(&[], utc_days(&entries), at("2026-10-19T00:00:00Z"), &Utc);
        let block = ArchivedBlock {
            start: at("2026-10-17T10:00:00Z"),
            end: at("2026-10-17T15:00:00Z"),
            first_request: at("2026-10-17T10:00:00Z"),
            last_activity: at("2026-10-17T10:00:00Z"),
            usage: days[0].usage.clone(),
        };
        archive.update_usage(days, vec![block], &HashMap::new(), &pricing);
        archive.save();
        assert!(tmp.0.join(DAILY_FILE).exists());
        assert_eq!(fs::read_to_string(tmp.0.join(BLOCKS_FILE)).unwrap(), newer);
    }

    fn sample(minutes: i64, seven_day: f64) -> Sample {
        Sample {
            at: at("2026-10-18T00:00:00Z") + TimeDelta::minutes(minutes),
            five_hour: 0.0,
            seven_day,
        }
    }

    #[test]
    fn readings_join_by_time_and_resets_survive_thinning() {
        let now = at("2026-10-18T12:00:00Z");
        let mut archive = Archive::from_parts(Vec::new(), Vec::new());
        // Desktop's log, with a reset from 10 % to 1 %.
        archive.add_readings(
            "acct",
            &[sample(0, 10.0), sample(15, 1.0), sample(30, 2.0)],
            now,
        );
        // Later it holds only newer samples, one of them revised: Desktop's
        // value wins, the archive keeps the rest.
        archive.add_readings("acct", &[sample(30, 2.5), sample(45, 3.0)], now);
        let values = |archive: &Archive, now| -> Vec<f64> {
            archive
                .recent_samples("acct", now)
                .iter()
                .map(|s| s.seven_day)
                .collect()
        };
        assert_eq!(values(&archive, now), [10.0, 1.0, 2.5, 3.0]);
        // A drop across a gap (Desktop closed) fell somewhere in between.
        archive.add_readings("acct", &[sample(45, 3.0), sample(600, 0.5)], now);
        let account = archive.account("acct").unwrap();
        assert!(
            account
                .readings
                .iter()
                .all(|r| r.source == Source::Official)
        );
        let resets: Vec<_> = account.resets.iter().map(|r| (r.after, r.source)).collect();
        assert_eq!(
            resets,
            [
                (sample(15, 0.0).at, Source::Official),
                (sample(600, 0.0).at, Source::Estimate)
            ]
        );
        // Another account stays apart.
        archive.add_readings("other", &[sample(700, 50.0)], now);
        assert_eq!(values(&archive, now).len(), 5);
        assert_eq!(archive.latest_account(), Some("other"));

        // Two months on, old readings are thinned to one per hour, keeping
        // those around each reset; the estimate reads the last eight days.
        let later = now + TimeDelta::days(60);
        let newest = Sample {
            at: later,
            five_hour: 0.0,
            seven_day: 4.0,
        };
        archive.add_readings("acct", &[newest], later);
        let kept: Vec<_> = archive
            .account("acct")
            .unwrap()
            .readings
            .iter()
            .map(|r| r.at)
            .collect();
        let expected = [0, 15, 45, 600].map(|m| sample(m, 0.0).at);
        assert_eq!(kept[..4], expected);
        assert_eq!(kept.len(), 5);
        assert_eq!(values(&archive, later), [0.5, 4.0]);
    }

    #[test]
    fn reading_times_tell_who_was_signed_in_as_the_last_eight_days_began() {
        let mut archive = Archive::from_parts(Vec::new(), Vec::new());
        let now = sample(0, 0.0).at + RECENT_READINGS;
        // a, then b just before the eight days, a again within them.
        archive.add_readings("a", &[sample(-30, 1.0), sample(1, 1.0)], now);
        archive.add_readings("b", &[sample(-25, 1.0), sample(-10, 1.0)], now);
        let mut times = archive.reading_times(now);
        times.sort_unstable();
        let at = |minutes| sample(minutes, 0.0).at;
        assert_eq!(times, [(at(-10), "b"), (at(1), "a")]);
        // So b's requests before a's return are b's, however they began.
        assert_eq!(
            quota::foreign_spans(times, "a"),
            [(DateTime::<Utc>::MIN_UTC, at(1))]
        );
    }

    #[test]
    fn calibration_keeps_the_latest_fit_per_day_until_it_is_a_week_old() {
        let mut archive = Archive::from_parts(Vec::new(), Vec::new());
        archive.add_readings("acct", &[sample(0, 1.0)], at("2026-10-18T00:00:00Z"));
        let fit = |five_hour, seven_day| PercentPerUsd {
            five_hour,
            seven_day,
        };
        archive.record_calibration("acct", fit(Some(2.0), None), at("2026-10-18T10:00:00Z"));
        archive.record_calibration(
            "acct",
            fit(Some(2.5), Some(0.3)),
            at("2026-10-18T11:00:00Z"),
        );
        archive.record_calibration(
            "acct",
            fit(Some(3.0), Some(0.3)),
            at("2026-10-19T09:00:00Z"),
        );
        // A refit that only wobbles in its last digits is the same fit.
        archive.weekly_slot.dirty = false;
        let wobble = 3.0 * (1.0 + 1e-12);
        archive.record_calibration(
            "acct",
            fit(Some(wobble), Some(0.3)),
            at("2026-10-19T09:00:00Z"),
        );
        assert!(!archive.weekly_slot.dirty);
        let account = archive.account("acct").unwrap();
        let fits: Vec<_> = account
            .calibration
            .iter()
            .map(|c| (c.window, c.percent_per_usd))
            .collect();
        assert_eq!(
            fits,
            [
                (Window::FiveHour, 2.5),
                (Window::SevenDay, 0.3),
                (Window::FiveHour, 3.0),
                (Window::SevenDay, 0.3)
            ]
        );
        assert!(
            account
                .calibration
                .iter()
                .all(|c| c.source == Source::Estimate)
        );
        assert_eq!(
            archive.calibration("acct", at("2026-10-20T00:00:00Z")),
            fit(Some(3.0), Some(0.3))
        );
        assert_eq!(
            archive.calibration("acct", at("2026-10-27T00:00:00Z")),
            fit(None, None),
            "too old to stand in"
        );
        assert_eq!(
            archive.calibration("nobody", at("2026-10-20T00:00:00Z")),
            fit(None, None)
        );

        // A fit of spend weighted otherwise does not stand in.
        let mut legacy = Archive::from_parts(Vec::new(), Vec::new());
        legacy.add_readings("acct", &[sample(0, 1.0)], at("2026-10-18T00:00:00Z"));
        legacy.weekly.accounts[0].calibration.push(Calibration {
            at: at("2026-10-19T09:00:00Z"),
            window: Window::FiveHour,
            percent_per_usd: 2.5,
            weights: None,
            source: Source::Estimate,
        });
        let now = at("2026-10-20T00:00:00Z");
        assert_eq!(legacy.calibration("acct", now), fit(None, None));
        // Refitting the same readings replaces it.
        legacy.record_calibration("acct", fit(Some(2.5), None), at("2026-10-19T09:00:00Z"));
        assert_eq!(legacy.calibration("acct", now), fit(Some(2.5), None));
    }
}
