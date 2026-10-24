//! Deduplication, 5-hour rolling blocks, and the rollups sent to the frontend,
//! merged with the archive of usage whose transcripts are gone.

#[cfg(test)]
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chrono::{
    DateTime, Datelike, Days, DurationRound, Months, NaiveDate, Offset, TimeDelta, TimeZone, Utc,
};
use serde::{Deserialize, Serialize};

use super::archive::{Archive, Row, session_tag};
use super::pricing::Pricing;
use super::quota::Span;
use super::types::{
    ActiveBlock, BlockBucket, BlockRecord, BurnRate, DailyUsage, ModelUsage, MonthlyUsage,
    PastBlock, ProjectUsage, Projection, TokenCounts, UsageData, UsageEntry, UsageStats,
};

/// Days of the daily series in every snapshot: the 14-day chart, the Mini
/// HUD and the comparison with yesterday read no further back, and a
/// snapshot that grows by a row a day would grow without end. The archive
/// view fetches every day (`History::daily_history`).
const RECENT_DAYS: u64 = 31;
/// Months of the monthly series in every snapshot, as many as the chart shows.
const RECENT_MONTHS: u32 = 12;

const BLOCK_HOURS: i64 = 5;
/// Claude starts its window at the first request rounded down to this many
/// minutes. Seen in its usage panel: first requests at 21:07, 07:37 and 20:42
/// reset at 02:00, 12:30 and 01:40, which only 10 fits (5 gives 12:35, 15 gives
/// 01:30); Desktop's readings agree (a window opened at 21:51:58 ended between
/// 02:41 and 02:56).
const BLOCK_ALIGN_MINUTES: i64 = 10;
/// Resolution of the active block's timeline (30 buckets per block).
const BUCKET_MINUTES: i64 = 10;
const BUCKET_COUNT: usize = (BLOCK_HOURS * 60 / BUCKET_MINUTES) as usize;
/// Ended blocks the projection learns the rest of a block from. On the
/// author's logs, the median rest of past blocks was off by 10 to 14% on
/// average (by 56 to 66% assuming the pace so far lasts until the reset,
/// since use usually stops hours before it).
const PROJECTION_BLOCKS: usize = 20;

fn block_duration() -> TimeDelta {
    TimeDelta::hours(BLOCK_HOURS)
}

/// Display info for a project folder, resolved from the `cwd` in its logs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Accum {
    tokens: TokenCounts,
    cost: f64,
    count: u64,
    unpriced_count: u64,
}

impl Accum {
    fn add(&mut self, entry: &UsageEntry) {
        self.tokens += entry.tokens;
        self.cost += entry.cost_usd;
        self.count += 1;
        self.unpriced_count += u64::from(!entry.priced);
    }

    fn remove(&mut self, entry: &UsageEntry) {
        self.tokens.input -= entry.tokens.input;
        self.tokens.output -= entry.tokens.output;
        self.tokens.cache_creation -= entry.tokens.cache_creation;
        self.tokens.cache_read -= entry.tokens.cache_read;
        self.cost -= entry.cost_usd;
        self.count -= 1;
        self.unpriced_count -= u64::from(!entry.priced);
        if self.count == 0 {
            *self = Self::default();
        }
    }

    fn merge(&mut self, other: &Self) {
        self.tokens += other.tokens;
        self.cost += other.cost;
        self.count += other.count;
        self.unpriced_count += other.unpriced_count;
    }

    fn add_row(&mut self, row: &Row, priced: bool) {
        self.tokens += row.tokens();
        self.cost += row.cost_usd;
        self.count += row.responses;
        self.unpriced_count += if priced { 0 } else { row.responses };
    }

    /// What `self` holds beyond `other`, never below zero.
    fn missing(&self, other: &Self) -> Self {
        Self {
            tokens: self.tokens.saturating_sub(&other.tokens),
            cost: (self.cost - other.cost).max(0.0),
            count: self.count.saturating_sub(other.count),
            unpriced_count: self.unpriced_count.saturating_sub(other.unpriced_count),
        }
    }

    fn stats(&self) -> UsageStats {
        let prompt = self.tokens.prompt();
        UsageStats {
            input_tokens: self.tokens.input,
            output_tokens: self.tokens.output,
            cache_creation_tokens: self.tokens.cache_creation,
            cache_read_tokens: self.tokens.cache_read,
            total_tokens: self.tokens.total(),
            cost_usd: self.cost,
            message_count: self.count,
            unpriced_message_count: self.unpriced_count,
            cache_hit_ratio: if prompt == 0 {
                0.0
            } else {
                self.tokens.cache_read as f64 / prompt as f64
            },
        }
    }
}

/// Keeps only the latest entry (by timestamp) per `message.id`, across all
/// files, and returns the survivors in chronological order.
#[cfg(test)]
pub fn dedup<'a>(entries: impl IntoIterator<Item = &'a UsageEntry>) -> Vec<&'a UsageEntry> {
    let mut latest: HashMap<&str, &UsageEntry> = HashMap::new();
    for entry in entries {
        match latest.entry(&*entry.dedup_key) {
            Entry::Occupied(mut slot) => {
                if entry.precedence(slot.get()).is_gt() {
                    slot.insert(entry);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(entry);
            }
        }
    }
    let mut sorted: Vec<_> = latest.into_values().collect();
    sorted.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.dedup_key.cmp(&b.dedup_key))
    });
    sorted
}

/// A 5-hour block: `[start, start + 5h]`, both ends inclusive.
#[derive(Debug, Clone)]
pub struct Block {
    /// `requested_at` of the message that opened the block.
    pub opened: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    acc: Accum,
    /// From the archive: it stands in for the responses requested within it.
    archived: bool,
}

impl Block {
    /// The first request rounded down to `BLOCK_ALIGN_MINUTES`, as Claude
    /// starts its own window.
    pub fn start(&self) -> DateTime<Utc> {
        self.opened
            .duration_trunc(TimeDelta::minutes(BLOCK_ALIGN_MINUTES))
            .unwrap_or(self.opened)
    }

    pub fn end(&self) -> DateTime<Utc> {
        self.start() + block_duration()
    }
}

/// CLAUDE.md §3.1: the first request opens a block at T0 (its 10 minutes); every
/// request within `[T0, T0 + 5h]` joins it; the first one after opens the next.
/// `sorted` must be ordered by `requested_at`.
#[cfg(test)]
pub fn build_blocks(sorted: &[&UsageEntry]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for &entry in sorted {
        push_block(&mut blocks, entry);
    }
    blocks
}

fn push_block(blocks: &mut Vec<Block>, entry: &UsageEntry) {
    match blocks.last_mut() {
        Some(block) if entry.requested_at <= block.end() => {
            block.acc.add(entry);
            block.last_activity = block.last_activity.max(entry.timestamp);
        }
        _ => {
            let mut acc = Accum::default();
            acc.add(entry);
            blocks.push(Block {
                opened: entry.requested_at,
                last_activity: entry.timestamp,
                acc,
                archived: false,
            });
        }
    }
}

/// Status of `block` at `now`, or `None` once the block has ended. `buckets`
/// contain its cached usage across the entire five-hour window.
///
/// Burn rate is measured over the time elapsed since the block's first
/// request (at least one minute; not since the hour it starts on, which would
/// count the idle minutes before it), so it decays while idle. The projection
/// adds `usual_rest`, what past blocks used after this far into them, or,
/// before any block has ended, the burn rate until the block ends.
fn active_block(
    block: &Block,
    buckets: &[Accum; BUCKET_COUNT],
    usual_rest: Option<Projection>,
    now: DateTime<Utc>,
) -> Option<ActiveBlock> {
    let (start, end) = (block.start(), block.end());
    if now > end {
        return None;
    }
    let elapsed_seconds = (now - start).num_seconds().max(0);
    let remaining_seconds = (end - now).num_seconds().max(0);
    let active_minutes = ((now - block.opened).num_seconds() as f64 / 60.0).max(1.0);
    let remaining_minutes = remaining_seconds as f64 / 60.0;

    let usage = block.acc.stats();
    let tokens_per_minute = usage.total_tokens as f64 / active_minutes;
    let cost_per_hour = usage.cost_usd / active_minutes * 60.0;

    Some(ActiveBlock {
        start_time: start,
        end_time: end,
        last_activity: block.last_activity,
        elapsed_seconds,
        remaining_seconds,
        projected: match usual_rest {
            Some(rest) => Projection {
                total_tokens: usage.total_tokens + rest.total_tokens,
                cost_usd: usage.cost_usd + rest.cost_usd,
            },
            None => Projection {
                total_tokens: usage.total_tokens
                    + (tokens_per_minute * remaining_minutes).round() as u64,
                cost_usd: usage.cost_usd + cost_per_hour * remaining_minutes / 60.0,
            },
        },
        burn_rate: BurnRate {
            tokens_per_minute,
            cost_per_hour,
        },
        usage,
        bucket_minutes: BUCKET_MINUTES,
        timeline: block_timeline(start, buckets, elapsed_seconds),
    })
}

/// Median usage after `elapsed` into each of the latest `PROJECTION_BLOCKS`
/// ended blocks (by response time), tokens and cost each on its own, or `None`
/// without any. `ended` yields each block's start and responses, latest first.
fn usual_rest<'a, I>(
    ended: impl Iterator<Item = (DateTime<Utc>, I)>,
    elapsed: TimeDelta,
) -> Option<Projection>
where
    I: IntoIterator<Item = &'a UsageEntry>,
{
    let (mut tokens, mut costs): (Vec<u64>, Vec<f64>) = ended
        .take(PROJECTION_BLOCKS)
        .map(|(start, entries)| {
            let mut rest = Accum::default();
            for entry in entries {
                if entry.timestamp > start + elapsed {
                    rest.add(entry);
                }
            }
            (rest.tokens.total(), rest.cost)
        })
        .unzip();
    let middle = tokens.len().checked_sub(1)? / 2;
    let even = tokens.len() % 2 == 0;
    tokens.sort_unstable();
    costs.sort_unstable_by(f64::total_cmp);
    Some(Projection {
        total_tokens: if even {
            tokens[middle].midpoint(tokens[middle + 1])
        } else {
            tokens[middle]
        },
        cost_usd: if even {
            costs[middle].midpoint(costs[middle + 1])
        } else {
            costs[middle]
        },
    })
}

/// The block with the most tokens among those that had ended by `now`. The
/// open block is left out so the header can show it passing the record.
fn peak_block(blocks: &[Block], now: DateTime<Utc>) -> Option<PastBlock> {
    blocks
        .iter()
        .filter(|block| now > block.end())
        .max_by_key(|block| block.acc.tokens.total())
        .map(|block| PastBlock {
            start_time: block.start(),
            end_time: block.end(),
            usage: block.acc.stats(),
        })
}

/// Cache the whole block in `BUCKET_MINUTES` buckets. A message at exactly
/// `start + 5h` lands in the last bucket.
fn block_buckets<'a>(
    start: DateTime<Utc>,
    entries: impl IntoIterator<Item = &'a UsageEntry>,
) -> [Accum; BUCKET_COUNT] {
    let mut buckets = [Accum::default(); BUCKET_COUNT];
    for entry in entries {
        let index =
            ((entry.timestamp - start).num_seconds() / (BUCKET_MINUTES * 60)).max(0) as usize;
        buckets[index.min(BUCKET_COUNT - 1)].add(entry);
    }
    buckets
}

fn block_timeline(
    start: DateTime<Utc>,
    cached: &[Accum; BUCKET_COUNT],
    elapsed_seconds: i64,
) -> Vec<BlockBucket> {
    let bucket = TimeDelta::minutes(BUCKET_MINUTES);
    let bucket_seconds = bucket.num_seconds();
    let last = BUCKET_COUNT - 1;
    let current = ((elapsed_seconds / bucket_seconds) as usize).min(last);

    let mut buckets = vec![Accum::default(); current + 1];
    for (index, acc) in cached.iter().enumerate() {
        buckets[index.min(current)].merge(acc);
    }
    buckets
        .iter()
        .zip(0..)
        .map(|(acc, i)| BlockBucket {
            start: start + bucket * i,
            usage: acc.stats(),
        })
        .collect()
}

#[cfg(test)]
struct ProjectAccum<'a> {
    acc: Accum,
    sessions: HashSet<&'a str>,
    last_activity: DateTime<Utc>,
}

/// Builds the full frontend payload. Calendar days ("today", the daily
/// history) are taken in `tz`. `data_dirs` and `file_count` are left for the
/// caller to fill in.
#[cfg(test)]
pub fn compute<'a, Tz: TimeZone>(
    entries: impl IntoIterator<Item = &'a UsageEntry>,
    project_meta: &HashMap<Arc<str>, ProjectMeta>,
    now: DateTime<Utc>,
    tz: &Tz,
) -> UsageData {
    let sorted = dedup(entries);
    let today = now.with_timezone(tz).date_naive();
    let (yesterday, cut) = same_time_yesterday(now, tz);

    let mut total = Accum::default();
    let mut today_acc = Accum::default();
    let mut yesterday_acc = Accum::default();
    let mut daily: BTreeMap<NaiveDate, Accum> = BTreeMap::new();
    let mut by_project: HashMap<&Arc<str>, ProjectAccum<'_>> = HashMap::new();
    let mut by_model: HashMap<&str, (Accum, bool)> = HashMap::new();

    for &entry in &sorted {
        let day = entry.timestamp.with_timezone(tz).date_naive();
        total.add(entry);
        if day == today {
            today_acc.add(entry);
        }
        if day == yesterday && entry.timestamp <= cut {
            yesterday_acc.add(entry);
        }
        daily.entry(day).or_default().add(entry);

        let project = by_project
            .entry(&entry.project)
            .or_insert_with(|| ProjectAccum {
                acc: Accum::default(),
                sessions: HashSet::new(),
                last_activity: entry.timestamp,
            });
        project.acc.add(entry);
        project.sessions.insert(entry.session_id.as_str());
        project.last_activity = entry.timestamp; // `sorted` is chronological

        by_model
            .entry(entry.model.as_str())
            .or_insert((Accum::default(), entry.priced))
            .0
            .add(entry);
    }

    let mut by_request = sorted.clone();
    by_request.sort_by(|a, b| {
        a.requested_at
            .cmp(&b.requested_at)
            .then_with(|| a.dedup_key.cmp(&b.dedup_key))
    });
    let blocks = build_blocks(&by_request);
    // Each block holds every message from its opening request to the next.
    let members = |index: usize| {
        let from =
            |block: &Block| by_request.partition_point(|entry| entry.requested_at < block.opened);
        let until = blocks.get(index + 1).map_or(by_request.len(), from);
        by_request[from(&blocks[index])..until].iter().copied()
    };
    let active_block = blocks.last().and_then(|block| {
        let ended = (0..blocks.len())
            .rev()
            .filter(|&index| now > blocks[index].end())
            .map(|index| (blocks[index].start(), members(index)));
        active_block(
            block,
            &block_buckets(block.start(), members(blocks.len() - 1)),
            usual_rest(ended, now - block.start()),
            now,
        )
    });

    let mut projects: Vec<ProjectUsage> = by_project
        .into_iter()
        .map(|(id, p)| {
            let meta = project_meta.get(id);
            ProjectUsage {
                id: id.to_string(),
                name: meta.map_or_else(|| id.to_string(), |m| m.name.clone()),
                path: meta.and_then(|m| m.path.clone()),
                session_count: p.sessions.len(),
                last_activity: p.last_activity,
                usage: p.acc.stats(),
            }
        })
        .collect();
    projects.sort_by(|a, b| {
        b.usage
            .total_tokens
            .cmp(&a.usage.total_tokens)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut models: Vec<ModelUsage> = by_model
        .into_iter()
        .map(|(model, (acc, priced))| ModelUsage {
            model: model.to_owned(),
            pricing_known: priced,
            usage: acc.stats(),
        })
        .collect();
    models.sort_by(|a, b| {
        b.usage
            .cost_usd
            .total_cmp(&a.usage.cost_usd)
            .then_with(|| b.usage.total_tokens.cmp(&a.usage.total_tokens))
            .then_with(|| a.model.cmp(&b.model))
    });

    let calendar = Calendar::new(&daily, today);
    UsageData {
        snapshot_id: 0,
        generated_at: now,
        data_dirs: Vec::new(),
        file_count: 0,
        custom_prices: 0,
        pricing_error: None,
        read_errors: Vec::new(),
        today: today_acc.stats(),
        yesterday_so_far: yesterday_acc.stats(),
        active_block,
        peak_block: peak_block(&blocks, now),
        daily: calendar.daily,
        monthly: calendar.monthly,
        first_day: calendar.first_day,
        projects,
        models,
        total: total.stats(),
        cache_read_price: None,
        quota: None,
        archive: None,
    }
}

/// Yesterday's date and when its clock read what it reads now. Only reads
/// UTC as local time: a clock time that a zone skips has no instant.
fn same_time_yesterday<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> (NaiveDate, DateTime<Utc>) {
    let offset = |at: DateTime<Utc>| {
        TimeDelta::seconds(at.with_timezone(tz).offset().fix().local_minus_utc().into())
    };
    let day_ago = now - TimeDelta::days(1);
    let today = now.with_timezone(tz).date_naive();
    (
        today.pred_opt().unwrap_or(NaiveDate::MIN),
        day_ago + offset(now) - offset(day_ago),
    )
}

/// One row per day from the first recorded day (or `from`, if later)
/// through today, zero-filled.
fn daily_series(
    daily: &BTreeMap<NaiveDate, Accum>,
    today: NaiveDate,
    from: Option<NaiveDate>,
) -> Vec<DailyUsage> {
    let (Some((&first, _)), Some((&last, _))) = (daily.first_key_value(), daily.last_key_value())
    else {
        return Vec::new();
    };
    let first = from.map_or(first, |from| first.max(from));
    let last = last.max(today);
    first
        .iter_days()
        .take_while(|day| *day <= last)
        .map(|day| DailyUsage {
            date: day.format("%Y-%m-%d").to_string(),
            usage: daily.get(&day).copied().unwrap_or_default().stats(),
        })
        .collect()
}

/// One row per month from the first recorded month (or the month of `from`,
/// if later) through this one, zero-filled.
fn monthly_series(
    daily: &BTreeMap<NaiveDate, Accum>,
    today: NaiveDate,
    from: Option<NaiveDate>,
) -> Vec<MonthlyUsage> {
    let (Some((&first, _)), Some((&last, _))) = (daily.first_key_value(), daily.last_key_value())
    else {
        return Vec::new();
    };
    let month_of = |day: NaiveDate| day.with_day(1).expect("every month has a 1st");
    let start = month_of(from.map_or(first, |from| first.max(from)));
    let mut months: BTreeMap<NaiveDate, Accum> = BTreeMap::new();
    for (day, acc) in daily.range(start..) {
        months.entry(month_of(*day)).or_default().merge(acc);
    }
    let end = month_of(last.max(today));
    std::iter::successors(Some(start), |month| {
        month
            .checked_add_months(Months::new(1))
            .filter(|next| *next <= end)
    })
    .map(|month| MonthlyUsage {
        month: month.format("%Y-%m").to_string(),
        usage: months.get(&month).copied().unwrap_or_default().stats(),
    })
    .collect()
}

/// The calendar series of a snapshot.
struct Calendar {
    daily: Vec<DailyUsage>,
    monthly: Vec<MonthlyUsage>,
    first_day: Option<String>,
}

impl Calendar {
    /// The last `RECENT_DAYS` days and `RECENT_MONTHS` months through
    /// `today`, from per-day usage in local days.
    fn new(daily: &BTreeMap<NaiveDate, Accum>, today: NaiveDate) -> Self {
        let first_month = today
            .with_day(1)
            .and_then(|month| month.checked_sub_months(Months::new(RECENT_MONTHS - 1)));
        Self {
            daily: daily_series(
                daily,
                today,
                today.checked_sub_days(Days::new(RECENT_DAYS - 1)),
            ),
            monthly: monthly_series(daily, today, first_month),
            first_day: daily
                .first_key_value()
                .map(|(day, _)| day.format("%Y-%m-%d").to_string()),
        }
    }
}

#[derive(Default)]
struct ProjectTotals {
    acc: Accum,
    sessions: HashMap<String, usize>,
    activity: BTreeMap<DateTime<Utc>, usize>,
}

/// Usage of one project × model on an archived day.
#[derive(Debug, Default, Clone, Copy)]
struct Cell {
    acc: Accum,
    quota: f64,
}

impl Cell {
    fn add(&mut self, entry: &UsageEntry) {
        self.acc.add(entry);
        self.quota += entry.quota_usd;
    }
}

/// An archived day, prepared for comparison with the live responses.
struct ArchivedSpan {
    date: NaiveDate,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    cells: HashMap<(Arc<str>, String), Cell>,
    /// Last activity per project.
    projects: HashMap<Arc<str>, DateTime<Utc>>,
}

#[derive(Default)]
struct ArchivedSessions {
    tags: HashSet<String>,
    /// Sessions in older archives that have no stable tag yet.
    unlinked: u64,
    last_activity: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Archived {
    /// By start; they never overlap.
    days: Vec<ArchivedSpan>,
    /// By opening request, flagged `archived`.
    blocks: Vec<Block>,
    sessions: HashMap<Arc<str>, ArchivedSessions>,
    /// Only changes to responses requested before this can alter the merge.
    end: Option<DateTime<Utc>>,
}

impl Archived {
    fn date_of(&self, at: DateTime<Utc>) -> Option<NaiveDate> {
        let next = self.days.partition_point(|day| day.start <= at);
        next.checked_sub(1)
            .map(|index| &self.days[index])
            .filter(|day| at < day.end)
            .map(|day| day.date)
    }
}

#[derive(Default)]
struct FillProject {
    acc: Accum,
    sessions: usize,
    last_activity: Option<DateTime<Utc>>,
}

/// What the archive adds to the live rollups: the part of each archived day
/// × project × model whose responses the transcripts no longer hold.
#[derive(Default)]
struct Fill {
    daily: BTreeMap<NaiveDate, Accum>,
    projects: HashMap<Arc<str>, FillProject>,
    models: HashMap<String, (Accum, bool)>,
    total: Accum,
    /// Quota weight per archived day `[start, end)`, project and model;
    /// account attribution approximates missing request times by duration.
    quota: Vec<(Span, Arc<str>, String, f64)>,
}

/// One ended block of the chain, with its live responses (none for an
/// archived one, which stands in for responses no longer on disk).
pub struct EndedBlock<'a> {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub opened: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub archived: bool,
    pub entries: Vec<&'a UsageEntry>,
}

/// Incremental winners and rollups. File deletion/rewrite rebuilds this index;
/// ordinary appends only replace the affected message, day, project and model.
/// The ordered index lets late records rebuild only the affected block suffix.
/// An archive (`set_archive`) fills in what the transcripts no longer hold.
#[derive(Default)]
pub struct History {
    /// Each winner with the local day it was rolled up under, so removing it
    /// never depends on the zone rules still being the same.
    entries: HashMap<Arc<str>, (Arc<UsageEntry>, NaiveDate)>,
    /// Keyed by `(requested_at, dedup_key)`, the order blocks are built in.
    ordered: BTreeMap<(DateTime<Utc>, Arc<str>), Arc<UsageEntry>>,
    total: Accum,
    daily: BTreeMap<NaiveDate, Accum>,
    projects: HashMap<Arc<str>, ProjectTotals>,
    models: HashMap<String, (Accum, bool)>,
    blocks: Vec<Block>,
    blocks_dirty_from: Option<DateTime<Utc>>,
    buckets: [Accum; BUCKET_COUNT],
    /// `zone_fingerprint` of the zone `daily` was built in.
    zone: Option<[i32; 3]>,
    /// Installing an archive can change the date of an already indexed response.
    calendar_dirty: bool,
    archived: Archived,
    fill: Fill,
    fill_dirty: bool,
    /// Archived blocks whose responses are partly gone from the transcripts;
    /// each stands in for the responses requested within it.
    overrides: Vec<Block>,
    /// Earliest request time added or replaced since `take_earliest_change`.
    earliest_change: Option<DateTime<Utc>>,
    /// Spans another account than the one signed in now made the requests
    /// in (`set_foreign`), oldest first.
    foreign: Vec<Span>,
    /// While `foreign` holds any, the latest block of the account signed in
    /// now, which the current block and the quota go by instead of `blocks`.
    own: Option<OwnBlock>,
    own_dirty: bool,
}

/// The latest block of a chain over one account's requests (`None` before
/// its first) and its usage in `BUCKET_MINUTES` buckets.
struct OwnBlock {
    block: Option<Block>,
    buckets: [Accum; BUCKET_COUNT],
}

/// UTC offsets now and in mid-January and mid-July of this year. The current
/// offset alone misses a switch between zones that agree today but not in the
/// other half of the year (Denver and Phoenix in winter).
fn zone_fingerprint<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> [i32; 3] {
    let offset = |time: DateTime<Utc>| time.with_timezone(tz).offset().fix().local_minus_utc();
    let mid_month = |month| {
        Utc.with_ymd_and_hms(now.year(), month, 15, 12, 0, 0)
            .single()
            .map_or(0, offset)
    };
    [offset(now), mid_month(1), mid_month(7)]
}

impl History {
    /// The zone is re-checked by `snapshot`, not per entry.
    pub fn insert<Tz: TimeZone>(&mut self, entry: Arc<UsageEntry>, tz: &Tz) -> bool {
        if self
            .entries
            .get(&entry.dedup_key)
            .is_some_and(|(old, _)| !entry.precedence(old).is_gt())
        {
            return false;
        }
        if self.zone.is_none() {
            self.zone = Some(zone_fingerprint(Utc::now(), tz));
        }
        // A session can survive only in today's logs after its archived
        // responses are deleted. Its first live entry must refresh the union.
        if !self.archived.days.is_empty()
            && self
                .projects
                .get(&entry.project)
                .is_none_or(|p| !p.sessions.contains_key(&entry.session_id))
        {
            self.fill_dirty = true;
        }
        let mut first_changed = entry.requested_at;
        if let Some((old, day)) = self.entries.remove(&entry.dedup_key) {
            if old.project != entry.project || old.session_id != entry.session_id {
                self.fill_dirty = true;
            }
            first_changed = first_changed.min(old.requested_at);
            self.ordered
                .remove(&(old.requested_at, Arc::clone(&old.dedup_key)));
            self.remove_rollup(&old, day);
        }
        self.earliest_change = Some(
            self.earliest_change
                .map_or(first_changed, |t| t.min(first_changed)),
        );
        // A response is logged at or after its request, so a change from the
        // archive's end on cannot fall on an archived day or block.
        if self.archived.end.is_some_and(|end| first_changed < end) {
            self.fill_dirty = true;
        }
        let day = self
            .archived
            .date_of(entry.timestamp)
            .unwrap_or_else(|| entry.timestamp.with_timezone(tz).date_naive());
        self.add_rollup(&entry, day);
        self.blocks_dirty_from = Some(
            self.blocks_dirty_from
                .map_or(first_changed, |t| t.min(first_changed)),
        );
        self.ordered.insert(
            (entry.requested_at, Arc::clone(&entry.dedup_key)),
            Arc::clone(&entry),
        );
        self.entries
            .insert(Arc::clone(&entry.dedup_key), (entry, day));
        true
    }

    fn add_rollup(&mut self, entry: &UsageEntry, day: NaiveDate) {
        self.total.add(entry);
        self.daily.entry(day).or_default().add(entry);
        let project = self.projects.entry(Arc::clone(&entry.project)).or_default();
        project.acc.add(entry);
        *project
            .sessions
            .entry(entry.session_id.clone())
            .or_default() += 1;
        *project.activity.entry(entry.timestamp).or_default() += 1;
        self.models
            .entry(entry.model.clone())
            .or_insert((Accum::default(), entry.priced))
            .0
            .add(entry);
    }

    fn remove_rollup(&mut self, entry: &UsageEntry, day: NaiveDate) {
        self.total.remove(entry);
        let daily = self.daily.get_mut(&day).expect("indexed day");
        daily.remove(entry);
        if daily.count == 0 {
            self.daily.remove(&day);
        }
        let project = self
            .projects
            .get_mut(&entry.project)
            .expect("indexed project");
        project.acc.remove(entry);
        let sessions = project
            .sessions
            .get_mut(&entry.session_id)
            .expect("indexed session");
        *sessions -= 1;
        if *sessions == 0 {
            project.sessions.remove(&entry.session_id);
        }
        let activity = project
            .activity
            .get_mut(&entry.timestamp)
            .expect("indexed activity");
        *activity -= 1;
        if *activity == 0 {
            project.activity.remove(&entry.timestamp);
        }
        if project.acc.count == 0 {
            self.projects.remove(&entry.project);
        }
        let model = self.models.get_mut(&entry.model).expect("indexed model");
        model.0.remove(entry);
        if model.0.count == 0 {
            self.models.remove(&entry.model);
        }
    }

    fn check_timezone<Tz: TimeZone>(&mut self, now: DateTime<Utc>, tz: &Tz) {
        let fingerprint = zone_fingerprint(now, tz);
        if self.zone == Some(fingerprint) && !self.calendar_dirty {
            return;
        }
        // DST or a timezone switch can change calendar grouping, while UTC
        // ordering, block boundaries and non-calendar rollups remain valid.
        self.daily.clear();
        for (entry, day) in self.entries.values_mut() {
            *day = self
                .archived
                .date_of(entry.timestamp)
                .unwrap_or_else(|| entry.timestamp.with_timezone(tz).date_naive());
            self.daily.entry(*day).or_default().add(entry);
        }
        self.zone = Some(fingerprint);
        self.calendar_dirty = false;
    }

    /// Merges `archive` into every rollup from the next snapshot on, priced
    /// with `pricing`. Its days and blocks must not overlap.
    pub fn set_archive(&mut self, archive: &Archive, pricing: &Pricing) {
        let mut known: HashMap<String, bool> = HashMap::new();
        let mut priced = |model: &str| -> bool {
            if let Some(&priced) = known.get(model) {
                return priced;
            }
            let priced = pricing.rates_for(model).is_some();
            known.insert(model.to_owned(), priced);
            priced
        };
        let days: Vec<ArchivedSpan> = archive
            .days()
            .iter()
            .map(|day| {
                let mut cells: HashMap<(Arc<str>, String), Cell> = HashMap::new();
                for row in &day.usage {
                    let cell = cells
                        .entry((Arc::from(row.project.as_str()), row.model.clone()))
                        .or_default();
                    cell.acc.add_row(row, priced(&row.model));
                    cell.quota += row.quota_usd;
                }
                ArchivedSpan {
                    date: day.date,
                    start: day.start,
                    end: day.end,
                    cells,
                    projects: day
                        .projects
                        .iter()
                        .map(|p| (Arc::from(p.project.as_str()), p.last_activity))
                        .collect(),
                }
            })
            .collect();
        let blocks: Vec<Block> = archive
            .blocks()
            .iter()
            .map(|block| {
                let mut acc = Accum::default();
                for row in &block.usage {
                    acc.add_row(row, priced(&row.model));
                }
                Block {
                    opened: block.first_request,
                    last_activity: block.last_activity,
                    acc,
                    archived: true,
                }
            })
            .collect();
        let end = days
            .last()
            .map(|day| day.end)
            .into_iter()
            .chain(blocks.last().map(Block::end))
            .max();
        let mut sessions: HashMap<Arc<str>, ArchivedSessions> = HashMap::new();
        for project in archive.days().iter().flat_map(|day| &day.projects) {
            let total = sessions
                .entry(Arc::from(project.project.as_str()))
                .or_default();
            total.tags.extend(project.session_tags.iter().cloned());
            total.unlinked += project
                .sessions
                .saturating_sub(project.session_tags.len() as u64);
            total.last_activity = total.last_activity.max(Some(project.last_activity));
        }
        self.archived = Archived {
            days,
            blocks,
            sessions,
            end,
        };
        self.calendar_dirty = true;
        self.fill_dirty = true;
    }

    /// Compares the archive with the live responses. An archived day ×
    /// project × model whose responses are partly gone from the transcripts
    /// adds what is missing; session counts are a union across all days;
    /// an archived block that lost some stands in for all of its.
    fn refresh_fill(&mut self) {
        if !std::mem::take(&mut self.fill_dirty) {
            return;
        }
        let days = &self.archived.days;
        let day_of = |at: DateTime<Utc>| {
            let next = days.partition_point(|day| day.start <= at);
            (next > 0 && at < days[next - 1].end).then(|| next - 1)
        };
        let mut live: HashMap<(usize, &Arc<str>, &str), Cell> = HashMap::new();
        if let Some(last) = days.last() {
            // Responses are logged at or after their request.
            for entry in self
                .ordered
                .range(..(last.end, Arc::from("")))
                .map(|(_, entry)| entry)
            {
                if let Some(index) = day_of(entry.timestamp) {
                    live.entry((index, &entry.project, entry.model.as_str()))
                        .or_default()
                        .add(entry);
                }
            }
        }
        let mut fill = Fill::default();
        for (index, day) in days.iter().enumerate() {
            let mut filled = HashSet::new();
            for ((project, model), cell) in &day.cells {
                let have = live
                    .get(&(index, project, model.as_str()))
                    .copied()
                    .unwrap_or_default();
                if cell.acc.count <= have.acc.count {
                    continue;
                }
                let missing = cell.acc.missing(&have.acc);
                fill.daily.entry(day.date).or_default().merge(&missing);
                fill.projects
                    .entry(Arc::clone(project))
                    .or_default()
                    .acc
                    .merge(&missing);
                fill.models
                    .entry(model.clone())
                    .or_insert((Accum::default(), cell.acc.unpriced_count == 0))
                    .0
                    .merge(&missing);
                fill.total.merge(&missing);
                fill.quota.push((
                    (day.start, day.end),
                    Arc::clone(project),
                    model.clone(),
                    (cell.quota - have.quota).max(0.0),
                ));
                filled.insert(project);
            }
            for (project, &last) in &day.projects {
                if !filled.remove(project) {
                    continue;
                }
                let entry = fill.projects.entry(Arc::clone(project)).or_default();
                entry.last_activity = entry.last_activity.max(Some(last));
            }
            // Filled without a project row (an edited file): the day's start.
            for project in filled {
                let entry = fill.projects.entry(Arc::clone(project)).or_default();
                entry.last_activity = entry.last_activity.max(Some(day.start));
            }
        }

        for (project, archived) in &self.archived.sessions {
            let live = self.projects.get(project);
            let shared = live.map_or(0, |p| {
                p.sessions
                    .keys()
                    .filter(|id| archived.tags.contains(&session_tag(id)))
                    .count()
            });
            let unmatched_live = live.map_or(0, |p| p.sessions.len()) - shared;
            // Legacy counts cannot distinguish a surviving partial session
            // from a different one: use a lower bound instead of double counting.
            let extra = archived.tags.len() - shared
                + archived.unlinked.saturating_sub(unmatched_live as u64) as usize;
            if extra > 0 {
                let entry = fill.projects.entry(Arc::clone(project)).or_default();
                entry.sessions = extra;
                entry.last_activity = entry.last_activity.max(archived.last_activity);
            }
        }

        let overrides: Vec<Block> = self
            .archived
            .blocks
            .iter()
            .filter(|block| {
                let after_end = block.end() + TimeDelta::nanoseconds(1);
                let have = self
                    .ordered
                    .range((block.opened, Arc::from(""))..(after_end, Arc::from("")))
                    .count() as u64;
                block.acc.count > have
            })
            .cloned()
            .collect();
        self.fill = fill;
        self.overrides = overrides;
        // Rare (the archive or its past changed), so the chain is rebuilt.
        self.blocks_dirty_from = Some(DateTime::<Utc>::MIN_UTC);
    }

    fn refresh_blocks(&mut self) {
        let Some(changed) = self.blocks_dirty_from.take() else {
            return;
        };
        self.own_dirty = true;
        let keep = self.blocks.partition_point(|block| block.end() < changed);
        let from = self
            .blocks
            .get(keep)
            .map_or(changed, |block| block.opened.min(changed));
        self.blocks.truncate(keep);
        // Archived blocks that win join the chain where they opened and take
        // the responses requested within them.
        let first_override = self.overrides.partition_point(|block| block.opened < from);
        let mut overrides = self.overrides[first_override..].iter().peekable();
        for entry in self
            .ordered
            .range((from, Arc::from(""))..)
            .map(|(_, entry)| entry)
        {
            while let Some(block) = overrides.next_if(|block| block.opened <= entry.requested_at) {
                self.blocks.push(block.clone());
            }
            if self
                .blocks
                .last()
                .is_some_and(|block| block.archived && entry.requested_at <= block.end())
            {
                continue;
            }
            push_block(&mut self.blocks, entry);
        }
        self.blocks.extend(overrides.cloned());
        self.buckets = match self.blocks.last() {
            Some(block) if !block.archived => block_buckets(
                block.start(),
                self.ordered
                    .range((block.opened, Arc::from(""))..)
                    .map(|(_, e)| e.as_ref()),
            ),
            _ => [Accum::default(); BUCKET_COUNT],
        };
    }

    /// Requests within `spans` (oldest first) were another account's: from
    /// the next snapshot on, the current block and the quota's spend leave
    /// them out, so a switch carries neither a block nor its usage over.
    pub fn set_foreign(&mut self, spans: Vec<Span>) {
        if spans != self.foreign {
            self.foreign = spans;
            self.own_dirty = true;
        }
    }

    pub fn foreign(&self) -> &[Span] {
        &self.foreign
    }

    fn is_foreign(&self, requested_at: DateTime<Utc>) -> bool {
        let next = self
            .foreign
            .partition_point(|(start, _)| *start <= requested_at);
        next > 0 && requested_at < self.foreign[next - 1].1
    }

    /// The chain over the current account's requests agrees with `blocks`
    /// before the first foreign span; from the block open as it began (or
    /// from its end), it is built again without the foreign requests. An
    /// archived block after that is left out: its responses are on disk
    /// except when transcripts were deleted by hand.
    fn refresh_own(&mut self) {
        if !std::mem::take(&mut self.own_dirty) {
            return;
        }
        let Some(&(first, first_end)) = self.foreign.first() else {
            self.own = None;
            return;
        };
        let shared = self.blocks.partition_point(|block| block.opened < first);
        let mut chain: Vec<Block> = self.blocks[..shared].last().cloned().into_iter().collect();
        let from = match chain.last() {
            Some(block) if !block.archived && block.end() >= first => {
                let opened = block.opened;
                chain.clear();
                opened
            }
            _ => first_end,
        };
        for entry in self
            .ordered
            .range((from, Arc::from(""))..)
            .map(|(_, entry)| entry)
        {
            if self.is_foreign(entry.requested_at)
                || chain
                    .last()
                    .is_some_and(|block| block.archived && entry.requested_at <= block.end())
            {
                continue;
            }
            push_block(&mut chain, entry);
        }
        let block = chain.pop();
        let buckets = match &block {
            Some(block) if !block.archived => block_buckets(
                block.start(),
                self.ordered
                    .range((block.opened, Arc::from(""))..)
                    .map(|(_, e)| e.as_ref())
                    .filter(|e| !self.is_foreign(e.requested_at)),
            ),
            _ => [Accum::default(); BUCKET_COUNT],
        };
        self.own = Some(OwnBlock { block, buckets });
    }

    /// The latest block of the account signed in now, and its buckets.
    fn current_block(&self) -> (Option<&Block>, &[Accum; BUCKET_COUNT]) {
        match &self.own {
            Some(own) => (own.block.as_ref(), &own.buckets),
            None => (self.blocks.last(), &self.buckets),
        }
    }

    /// Brings the calendar, the archive fill and the block chains up to date.
    fn refresh<Tz: TimeZone>(&mut self, now: DateTime<Utc>, tz: &Tz) {
        self.check_timezone(now, tz);
        self.refresh_fill();
        self.refresh_blocks();
        self.refresh_own();
    }

    pub fn snapshot<Tz: TimeZone>(
        &mut self,
        meta: &HashMap<Arc<str>, ProjectMeta>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> UsageData {
        self.refresh(now, tz);
        let today = now.with_timezone(tz).date_naive();
        let fill = &self.fill;
        let fill_only = fill
            .projects
            .keys()
            .filter(|id| !self.projects.contains_key(*id));
        let mut projects: Vec<_> = self
            .projects
            .keys()
            .chain(fill_only)
            .map(|id| {
                let info = meta.get(id);
                let live = self.projects.get(id);
                let filled = fill.projects.get(id);
                let mut acc = live.map_or_else(Accum::default, |p| p.acc);
                if let Some(filled) = filled {
                    acc.merge(&filled.acc);
                }
                let last_live =
                    live.map(|p| *p.activity.last_key_value().expect("nonempty project").0);
                ProjectUsage {
                    id: id.to_string(),
                    name: info.map_or_else(|| id.to_string(), |m| m.name.clone()),
                    path: info.and_then(|m| m.path.clone()),
                    session_count: live.map_or(0, |p| p.sessions.len())
                        + filled.map_or(0, |f| f.sessions),
                    last_activity: last_live
                        .max(filled.and_then(|f| f.last_activity))
                        .expect("live or filled activity"),
                    usage: acc.stats(),
                }
            })
            .collect();
        projects.sort_by(|a, b| {
            b.usage
                .total_tokens
                .cmp(&a.usage.total_tokens)
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut by_model: HashMap<&str, (Accum, bool)> = self
            .models
            .iter()
            .map(|(model, &(acc, known))| (model.as_str(), (acc, known)))
            .collect();
        for (model, (acc, known)) in &fill.models {
            by_model
                .entry(model)
                .or_insert((Accum::default(), *known))
                .0
                .merge(acc);
        }
        let mut models: Vec<_> = by_model
            .into_iter()
            .map(|(model, (acc, known))| ModelUsage {
                model: model.to_owned(),
                pricing_known: known,
                usage: acc.stats(),
            })
            .collect();
        models.sort_by(|a, b| {
            b.usage
                .cost_usd
                .total_cmp(&a.usage.cost_usd)
                .then_with(|| b.usage.total_tokens.cmp(&a.usage.total_tokens))
                .then_with(|| a.model.cmp(&b.model))
        });
        let daily = self.merged_daily();
        let calendar = Calendar::new(&daily, today);
        let mut total = self.total;
        total.merge(&fill.total);
        let (current, buckets) = self.current_block();
        UsageData {
            snapshot_id: 0,
            generated_at: now,
            data_dirs: Vec::new(),
            file_count: 0,
            custom_prices: 0,
            pricing_error: None,
            read_errors: Vec::new(),
            today: daily.get(&today).copied().unwrap_or_default().stats(),
            yesterday_so_far: self.yesterday_so_far(now, tz),
            active_block: current.and_then(|block| {
                let ended = (0..self.blocks.len())
                    .rev()
                    .filter(|&index| {
                        let past = &self.blocks[index];
                        !past.archived && now > past.end() && past.end() < block.opened
                    })
                    .map(|index| (self.blocks[index].start(), self.members(index)));
                let rest = usual_rest(ended, now - block.start());
                active_block(block, buckets, rest, now)
            }),
            peak_block: peak_block(&self.blocks, now),
            daily: calendar.daily,
            monthly: calendar.monthly,
            first_day: calendar.first_day,
            projects,
            models,
            total: total.stats(),
            cache_read_price: None,
            quota: None,
            archive: None,
        }
    }

    /// Yesterday's live usage up to this time of day. Call after `refresh`.
    fn yesterday_so_far<Tz: TimeZone>(&self, now: DateTime<Utc>, tz: &Tz) -> UsageStats {
        let (yesterday, cut) = same_time_yesterday(now, tz);
        // Indexed by request time, which precedes the response by minutes.
        let from = cut - TimeDelta::days(2);
        let mut acc = Accum::default();
        for ((_, key), entry) in self
            .ordered
            .range((from, Arc::from(""))..(cut + TimeDelta::nanoseconds(1), Arc::from("")))
        {
            if self.entries[key].1 == yesterday && entry.timestamp <= cut {
                acc.add(entry);
            }
        }
        acc.stats()
    }

    /// Usage per local day, live and from the archive. Call after `refresh`.
    fn merged_daily(&self) -> BTreeMap<NaiveDate, Accum> {
        let mut daily = self.daily.clone();
        for (day, acc) in &self.fill.daily {
            daily.entry(*day).or_default().merge(acc);
        }
        daily
    }

    /// Every local day from the first with usage through today, zero-filled:
    /// the whole history that snapshots cut down to the recent days.
    pub fn daily_history<Tz: TimeZone>(&mut self, now: DateTime<Utc>, tz: &Tz) -> Vec<DailyUsage> {
        self.refresh(now, tz);
        let today = now.with_timezone(tz).date_naive();
        daily_series(&self.merged_daily(), today, None)
    }

    /// Responses logged after `since`, in request order.
    fn responses_after(&self, since: DateTime<Utc>) -> impl Iterator<Item = &UsageEntry> {
        // Indexed by request time, which precedes the response by minutes.
        // `since` comes from Desktop's samples, which could name any time.
        let from = since
            .checked_sub_signed(TimeDelta::hours(1))
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        self.ordered
            .range((from, Arc::from(""))..)
            .map(|(_, entry)| entry.as_ref())
            .filter(move |entry| entry.timestamp > since)
    }

    /// `(response time, quota spend)` of every response after `since`, but
    /// those requested within a foreign span.
    pub fn quota_spend_since(&self, since: DateTime<Utc>) -> Vec<(DateTime<Utc>, f64)> {
        self.responses_after(since)
            .filter(|entry| !self.is_foreign(entry.requested_at))
            .map(|entry| (entry.timestamp, entry.quota_usd))
            .collect()
    }

    /// Quota spend of the responses after `since` (but those requested within
    /// a foreign span), per project and per model, with the archived usage
    /// of whole days after it. Archived spend is approximated by the share
    /// of each day's duration outside foreign spans. Call after `snapshot`.
    pub fn quota_spend_by_source(
        &self,
        since: DateTime<Utc>,
    ) -> (HashMap<Arc<str>, f64>, HashMap<String, f64>) {
        let mut projects: HashMap<Arc<str>, f64> = HashMap::new();
        let mut models: HashMap<String, f64> = HashMap::new();
        for entry in self
            .responses_after(since)
            .filter(|entry| !self.is_foreign(entry.requested_at))
        {
            *projects.entry(Arc::clone(&entry.project)).or_default() += entry.quota_usd;
            *models.entry(entry.model.clone()).or_default() += entry.quota_usd;
        }
        for ((start, end), project, model, usd) in &self.fill.quota {
            if *start < since {
                continue;
            }
            let duration = (*end - *start).num_milliseconds();
            let foreign: i64 = self
                .foreign
                .iter()
                .map(|&(from, to)| (to.min(*end) - from.max(*start)).num_milliseconds().max(0))
                .sum();
            let own = (duration - foreign).max(0);
            if own > 0 {
                let usd = usd * (own as f64 / duration as f64);
                *projects.entry(Arc::clone(project)).or_default() += usd;
                *models.entry(model.clone()).or_default() += usd;
            }
        }
        (projects, models)
    }

    /// The latest local 5-hour block of the account signed in now, open or
    /// ended. Call after `snapshot`.
    pub fn last_block(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        self.current_block()
            .0
            .map(|block| (block.start(), block.end()))
    }

    /// End of the latest block that had ended by `now`. Call after `snapshot`.
    pub fn last_ended_block_end(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.blocks
            .iter()
            .rev()
            .map(Block::end)
            .find(|end| now > *end)
    }

    /// Every live response with its local day, in request order.
    pub fn entries(&self) -> impl Iterator<Item = (&UsageEntry, NaiveDate)> {
        self.ordered
            .iter()
            .map(|((_, key), entry)| (entry.as_ref(), self.entries[key].1))
    }

    /// The live responses requested within block `index` (none for an
    /// archived block, which stands in for them), in request order.
    fn members(&self, index: usize) -> impl Iterator<Item = &UsageEntry> {
        let block = &self.blocks[index];
        // Later responses went to the next block or were absorbed by it.
        let until = self.blocks.get(index + 1).map_or(block.end(), |next| {
            block.end().min(next.opened - TimeDelta::nanoseconds(1))
        });
        self.ordered
            .range((block.opened, Arc::from(""))..)
            .take_while(move |((requested_at, _), _)| !block.archived && *requested_at <= until)
            .map(|(_, entry)| entry.as_ref())
    }

    /// The blocks that had ended by `now`, oldest first, each with the live
    /// responses it holds. Call after `snapshot`.
    pub fn ended_blocks(&self, now: DateTime<Utc>) -> Vec<EndedBlock<'_>> {
        self.blocks
            .iter()
            .enumerate()
            .take_while(|(_, block)| now > block.end())
            .map(|(index, block)| EndedBlock {
                start: block.start(),
                end: block.end(),
                opened: block.opened,
                last_activity: block.last_activity,
                archived: block.archived,
                entries: self.members(index).collect(),
            })
            .collect()
    }

    /// The earliest request time of a response added or replaced since the
    /// last call.
    pub fn take_earliest_change(&mut self) -> Option<DateTime<Utc>> {
        self.earliest_change.take()
    }

    /// Responses counted from the archive alone. Call after `snapshot`.
    pub fn filled_responses(&self) -> u64 {
        self.fill.total.count
    }

    /// Usage counted from the archive alone and every block that had ended by
    /// `now`, oldest first.
    pub fn archive_view<Tz: TimeZone>(
        &mut self,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> (UsageStats, Vec<BlockRecord>) {
        self.refresh(now, tz);
        let blocks = self
            .blocks
            .iter()
            .filter(|block| now > block.end())
            .map(|block| BlockRecord {
                start_time: block.start(),
                opened_at: block.opened,
                last_activity: block.last_activity,
                total_tokens: block.acc.tokens.total(),
                cost_usd: block.acc.cost,
                message_count: block.acc.count,
                unpriced_message_count: block.acc.unpriced_count,
            })
            .collect();
        (self.fill.total.stats(), blocks)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::{FixedOffset, MappedLocalTime, NaiveDateTime};

    use super::*;

    fn at(ts: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn entry(key: &str, ts: &str, project: &str, input: u64) -> UsageEntry {
        UsageEntry {
            dedup_key: Arc::from(key),
            source: Arc::from(project),
            offset: 0,
            complete: true,
            timestamp: at(ts),
            requested_at: at(ts),
            model: "claude-opus-5-5".to_owned(),
            project: Arc::from(project),
            session_id: format!("{project}-session"),
            tokens: TokenCounts {
                input,
                ..Default::default()
            },
            cache_write_1h: 0,
            fast: false,
            long_context: false,
            cost_usd: input as f64 / 1000.0,
            quota_usd: input as f64 / 1000.0,
            priced: true,
        }
    }

    #[test]
    fn unpriced_responses_survive_all_rollups_and_dedup_replacements() {
        let known = entry("known", "2026-10-18T10:00:00Z", "p", 10);
        let mut unknown = entry("unknown", "2026-10-18T10:01:00Z", "p", 20);
        unknown.model = "custom-model".into();
        unknown.priced = false;
        unknown.cost_usd = 0.0;
        let now = at("2026-10-18T10:05:00Z");
        let mut history = History::default();
        history.insert(Arc::new(known), &Utc);
        history.insert(Arc::new(unknown.clone()), &Utc);
        let data = history.snapshot(&HashMap::new(), now, &Utc);
        for stats in [
            &data.total,
            &data.today,
            &data.daily[0].usage,
            &data.projects[0].usage,
            &data.active_block.as_ref().unwrap().usage,
            &data.active_block.as_ref().unwrap().timeline[0].usage,
        ] {
            assert_eq!(stats.unpriced_message_count, 1);
            assert_eq!(stats.message_count, 2);
            assert_eq!(stats.input_tokens, 30);
            assert_eq!(stats.cost_usd, 0.01);
        }
        let model = data
            .models
            .iter()
            .find(|model| model.model == "custom-model")
            .unwrap();
        assert!(!model.pricing_known);
        assert_eq!(model.usage.unpriced_message_count, 1);
        let ended = history.snapshot(&HashMap::new(), at("2026-10-18T16:00:00Z"), &Utc);
        assert_eq!(ended.peak_block.unwrap().usage.unpriced_message_count, 1);

        unknown.timestamp += TimeDelta::seconds(1);
        unknown.priced = true;
        unknown.model = "claude-opus-5-5".into();
        unknown.cost_usd = 0.02;
        history.insert(Arc::new(unknown), &Utc);
        let replaced = history.snapshot(&HashMap::new(), now, &Utc);
        assert_eq!(replaced.total.unpriced_message_count, 0);
        assert_eq!(replaced.total.message_count, 2);
        assert!((replaced.total.cost_usd - 0.03).abs() < 1e-10);
        assert_eq!(
            replaced.active_block.unwrap().usage.unpriced_message_count,
            0
        );
    }

    #[test]
    fn dedup_keeps_latest_entry_per_message_id() {
        let entries = [
            entry("msg_a", "2026-10-17T10:00:00Z", "p", 10),
            entry("msg_b", "2026-10-17T09:00:00Z", "p", 20),
            entry("msg_a", "2026-10-17T10:00:05Z", "p", 15),
            entry("msg_a", "2026-10-17T10:00:01Z", "q", 99),
        ];
        let kept = dedup(&entries);
        assert_eq!(kept.len(), 2);
        assert_eq!(&*kept[0].dedup_key, "msg_b");
        assert_eq!(kept[1].tokens.input, 15);
    }

    #[test]
    fn timestamp_ties_prefer_complete_then_stable_source_and_offset() {
        let mut partial = entry("same", "2026-10-17T10:00:00Z", "p", 1);
        partial.complete = false;
        partial.source = Arc::from("z-partial.jsonl");
        let mut complete = entry("same", "2026-10-17T10:00:00Z", "p", 100);
        complete.source = Arc::from("a-complete.jsonl");
        let mut later_line = complete.clone();
        later_line.offset = 42;
        later_line.tokens.input = 120;
        for order in [
            [&partial, &complete, &later_line],
            [&partial, &later_line, &complete],
            [&complete, &partial, &later_line],
            [&complete, &later_line, &partial],
            [&later_line, &partial, &complete],
            [&later_line, &complete, &partial],
        ] {
            assert_eq!(dedup(order)[0].tokens.input, 120);
        }
        let mut other_file = complete.clone();
        other_file.source = Arc::from("b-complete.jsonl");
        other_file.tokens.input = 150;
        assert_eq!(dedup([&complete, &other_file])[0].tokens.input, 150);
        assert_eq!(dedup([&other_file, &complete])[0].tokens.input, 150);
    }

    fn assert_payload(actual: &UsageData, expected: &UsageData) {
        fn compare(a: &serde_json::Value, b: &serde_json::Value) {
            match (a, b) {
                (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
                    let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                    assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "{a} != {b}");
                }
                (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                    assert_eq!(a.len(), b.len());
                    for (a, b) in a.iter().zip(b) {
                        compare(a, b);
                    }
                }
                (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                    assert_eq!(a.len(), b.len());
                    for (key, value) in a {
                        compare(value, &b[key]);
                    }
                }
                _ => assert_eq!(a, b),
            }
        }
        compare(
            &serde_json::to_value(actual).unwrap(),
            &serde_json::to_value(expected).unwrap(),
        );
    }

    #[test]
    fn incremental_history_matches_full_rollup_after_out_of_order_replacements() {
        let mut history = History::default();
        let mut entries = Vec::new();
        let meta = HashMap::new();
        let now = at("2026-10-17T16:00:00Z");
        let updates = [
            entry("a", "2026-10-17T10:00:00Z", "p", 10),
            entry("b", "2026-10-17T10:20:00Z", "p", 20),
            entry("c", "2026-10-17T15:00:00Z", "q", 30),
            entry("d", "2026-10-17T15:00:01Z", "q", 40),
            // Moving a block's first message must move subsequent boundaries.
            entry("a", "2026-10-17T10:05:00Z", "p", 50),
            entry("earlier", "2026-10-17T09:59:00Z", "r", 60),
            // A newer duplicate can change project, session and calendar day.
            entry("b", "2026-10-18T00:01:00Z", "other", 70),
        ];
        for entry in updates {
            history.insert(Arc::new(entry.clone()), &Utc);
            entries.push(entry);
            assert_payload(
                &history.snapshot(&meta, now, &Utc),
                &compute(&entries, &meta, now, &Utc),
            );
        }

        // Deterministic mixed older/newer updates exercise removal of empty
        // days/models/projects and session/activity reference counts.
        let mut seed = 7u64;
        for i in 0..250 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mut e = entry(
                &format!("m{}", seed % 31),
                "2026-10-17T00:00:00Z",
                &format!("p{}", seed % 5),
                seed % 100 + 1,
            );
            e.timestamp += TimeDelta::seconds((seed % 150_000) as i64);
            e.requested_at = e.timestamp - TimeDelta::seconds(((seed >> 20) % 90) as i64);
            e.model = format!("model-{}", seed % 3);
            e.source = Arc::from(format!("file-{}", seed % 4));
            e.session_id = format!("session-{}", seed % 9);
            e.cost_usd = (seed % 100) as f64 / 1024.0;
            e.offset = i;
            history.insert(Arc::new(e.clone()), &Utc);
            entries.push(e);
            assert_payload(
                &history.snapshot(&meta, now, &Utc),
                &compute(&entries, &meta, now, &Utc),
            );
        }
    }

    #[test]
    fn cached_history_updates_time_and_calendar_without_new_messages() {
        let e = entry("a", "2026-10-17T23:50:00Z", "p", 100);
        let mut history = History::default();
        history.insert(Arc::new(e.clone()), &Utc);
        let meta = HashMap::new();
        for time in [
            "2026-10-17T23:51:00Z",
            "2026-10-18T00:10:00Z",
            "2026-10-18T04:50:01Z",
        ] {
            let now = at(time);
            assert_payload(
                &history.snapshot(&meta, now, &Utc),
                &compute([&e], &meta, now, &Utc),
            );
        }
        let now = at("2026-10-18T00:10:00Z");
        let west = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_payload(
            &history.snapshot(&meta, now, &west),
            &compute([&e], &meta, now, &west),
        );
        let east = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        assert_payload(
            &history.snapshot(&meta, now, &east),
            &compute([&e], &meta, now, &east),
        );
    }

    static PHOENIX: AtomicBool = AtomicBool::new(false);

    /// Denver (UTC-6 from April to September, else UTC-7), or Phoenix (UTC-7
    /// all year) while `PHOENIX` is set. Both read UTC-7 in October.
    #[derive(Clone, Copy, Debug)]
    struct Mountain;

    impl TimeZone for Mountain {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Mountain
        }
        fn offset_from_local_date(&self, _: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            unimplemented!("only UTC -> local conversions are used")
        }
        fn offset_from_local_datetime(&self, _: &NaiveDateTime) -> MappedLocalTime<FixedOffset> {
            unimplemented!("only UTC -> local conversions are used")
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let dst = (4..=9).contains(&utc.month()) && !PHOENIX.load(Ordering::SeqCst);
            FixedOffset::west_opt(if dst { 6 } else { 7 } * 3600).unwrap()
        }
    }

    #[test]
    fn zone_switches_regroup_days_and_never_lose_a_replaced_entry() {
        // July lies months back, outside a snapshot's days: read them all.
        let days = |history: &mut History, now| -> Vec<String> {
            history
                .daily_history(now, &Mountain)
                .into_iter()
                .filter(|day| day.usage.message_count > 0)
                .map(|day| day.date)
                .collect()
        };
        let meta = HashMap::new();
        let october = at("2026-10-18T12:00:00Z");
        let mut history = History::default();
        // 00:30 on Jul 15 in Denver, 23:30 on Jul 14 in Phoenix.
        let first = entry("k", "2026-07-15T06:30:00Z", "p", 10);
        history.insert(Arc::new(first.clone()), &Mountain);
        // The zone changes, then the same message copied into another
        // transcript wins the tie: it must leave the day it was counted on.
        PHOENIX.store(true, Ordering::SeqCst);
        let mut copy = first;
        copy.source = Arc::from("q");
        history.insert(Arc::new(copy), &Mountain);
        let data = history.snapshot(&meta, october, &Mountain);
        assert_eq!(data.total.message_count, 1);
        assert_eq!(days(&mut history, october), ["2026-07-14"]);
        // Back to Denver: October's offset is the same, July's is not.
        PHOENIX.store(false, Ordering::SeqCst);
        history.snapshot(&meta, october, &Mountain);
        assert_eq!(days(&mut history, october), ["2026-07-15"]);
    }

    #[test]
    fn snapshots_carry_the_recent_days_and_months_and_the_archive_view_all_days() {
        let entries = [
            entry("old", "2025-06-10T10:00:00Z", "p", 5),
            entry("a", "2026-09-30T10:00:00Z", "p", 7),
            entry("b", "2026-10-18T10:00:00Z", "p", 11),
        ];
        let now = at("2026-10-18T12:00:00Z");
        let mut history = History::default();
        for e in &entries {
            history.insert(Arc::new(e.clone()), &Utc);
        }
        let data = history.snapshot(&HashMap::new(), now, &Utc);
        assert_payload(&data, &compute(&entries, &HashMap::new(), now, &Utc));
        assert_eq!(data.total.total_tokens, 23, "totals cover every day");
        assert_eq!(data.first_day.as_deref(), Some("2025-06-10"));
        let days: Vec<_> = data
            .daily
            .iter()
            .map(|d| (d.date.as_str(), d.usage.total_tokens))
            .collect();
        assert_eq!(days.len(), RECENT_DAYS as usize);
        assert_eq!(days[0], ("2026-09-18", 0));
        assert_eq!(days[12], ("2026-09-30", 7));
        assert_eq!(days[30], ("2026-10-18", 11));
        let months: Vec<_> = data
            .monthly
            .iter()
            .map(|m| (m.month.as_str(), m.usage.total_tokens))
            .collect();
        assert_eq!(months.len(), RECENT_MONTHS as usize);
        assert_eq!(months[0], ("2025-11", 0));
        assert_eq!(months[10..], [("2026-09", 7), ("2026-10", 11)]);

        let all = history.daily_history(now, &Utc);
        let span = (now.date_naive() - NaiveDate::from_ymd_opt(2025, 6, 10).unwrap()).num_days();
        assert_eq!(all.len(), span as usize + 1);
        assert_eq!(
            (all[0].date.as_str(), all[0].usage.total_tokens),
            ("2025-06-10", 5)
        );
        assert_eq!(all.iter().map(|d| d.usage.total_tokens).sum::<u64>(), 23);

        // A young history is sent whole.
        let young = compute(&entries[1..], &HashMap::new(), now, &Utc);
        assert_eq!(young.daily.len(), 19);
        assert_eq!(young.daily[0].date, "2026-09-30");
        let months: Vec<_> = young.monthly.iter().map(|m| m.month.as_str()).collect();
        assert_eq!(months, ["2026-09", "2026-10"]);
        let empty = compute(&entries[..0], &HashMap::new(), now, &Utc);
        assert!(empty.daily.is_empty() && empty.monthly.is_empty() && empty.first_day.is_none());
    }

    #[test]
    #[ignore = "synthetic performance benchmark; run with --release --ignored --nocapture benchmark_history"]
    fn benchmark_history() {
        use chrono::Local;
        use std::time::Instant;
        let now = Utc::now();
        let meta = HashMap::new();
        for count in [100_000usize, 500_000] {
            let entries: Vec<_> = (0..count)
                .map(|i| {
                    let mut e = entry(&format!("m{i}"), "2026-10-17T00:00:00Z", "p", 100);
                    e.timestamp = now - TimeDelta::seconds((count - i) as i64 * 30);
                    e
                })
                .collect();
            let start = Instant::now();
            let mut history = History::default();
            for e in &entries {
                history.insert(Arc::new(e.clone()), &Local);
            }
            std::hint::black_box(history.snapshot(&meta, now, &Local));
            let build = start.elapsed().as_secs_f64() * 1000.0;
            let mut full = Vec::new();
            let mut idle = Vec::new();
            let mut append = Vec::new();
            for i in 0..5 {
                let start = Instant::now();
                std::hint::black_box(compute(&entries, &meta, now, &Local));
                full.push(start.elapsed().as_secs_f64() * 1000.0);
                let start = Instant::now();
                std::hint::black_box(history.snapshot(&meta, now, &Local));
                idle.push(start.elapsed().as_secs_f64() * 1000.0);
                let mut e = entries.last().unwrap().clone();
                e.dedup_key = format!("new-{i}").into();
                e.timestamp = now;
                let start = Instant::now();
                history.insert(Arc::new(e), &Local);
                std::hint::black_box(history.snapshot(&meta, now, &Local));
                append.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            // Another account's span before every request: the current
            // account's chain is built again over all of them on each append.
            history.set_foreign(vec![(
                at("2026-10-16T23:00:00Z"),
                at("2026-10-17T00:00:00Z"),
            )]);
            let mut switched = Vec::new();
            for i in 0..5 {
                let mut e = entries.last().unwrap().clone();
                e.dedup_key = format!("switched-{i}").into();
                e.timestamp = now;
                let start = Instant::now();
                history.insert(Arc::new(e), &Local);
                std::hint::black_box(history.snapshot(&meta, now, &Local));
                switched.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            for times in [&mut full, &mut idle, &mut append, &mut switched] {
                times.sort_by(f64::total_cmp);
            }
            println!(
                "entries={count} build_ms={build:.1} full_ms={:.3} cached_tick_ms={:.3} append_and_snapshot_ms={:.3} after_switch_ms={:.3}",
                full[2], idle[2], append[2], switched[2]
            );
        }
    }

    #[test]
    fn blocks_include_the_5h_boundary_and_split_after_it() {
        let entries = [
            entry("a", "2026-10-17T08:13:00Z", "p", 1), // T0 = 08:10
            entry("b", "2026-10-17T12:00:00Z", "p", 1),
            entry("c", "2026-10-17T13:10:00Z", "p", 1), // exactly T0 + 5h
            entry("d", "2026-10-17T13:10:01Z", "p", 1), // first message after T0 + 5h
            entry("e", "2026-10-17T20:00:30Z", "p", 1),
        ];
        let sorted = dedup(&entries);
        let blocks = build_blocks(&sorted);
        let starts: Vec<_> = blocks.iter().map(|b| (b.start(), b.acc.count)).collect();
        assert_eq!(
            starts,
            vec![
                (at("2026-10-17T08:10:00Z"), 3),
                (at("2026-10-17T13:10:00Z"), 1),
                (at("2026-10-17T20:00:00Z"), 1),
            ]
        );
    }

    #[test]
    fn blocks_open_at_the_10_minutes_of_the_first_request() {
        // Seen in Claude's usage panel (UTC-4): first requests at 21:07, 07:37
        // and 20:42 reset at 02:00, 12:30 and 01:40.
        let mut first = entry("a", "2026-10-19T01:07:31.772Z", "p", 1);
        first.requested_at = at("2026-10-19T01:07:29.025Z");
        // Sent before the reset but answered after it: still the same block.
        let mut late = entry("b", "2026-10-19T06:00:05Z", "p", 1);
        late.requested_at = at("2026-10-19T05:59:58Z");
        let mut after = entry("c", "2026-10-19T11:37:51Z", "p", 1);
        after.requested_at = at("2026-10-19T11:37:46Z");
        let mut evening = entry("d", "2026-10-20T00:42:55Z", "p", 1);
        evening.requested_at = at("2026-10-20T00:42:51Z");

        let now = at("2026-10-19T04:10:20Z");
        let data = compute([&first], &HashMap::new(), now, &Utc);
        let block = data.active_block.expect("block still open");
        assert_eq!(block.start_time, at("2026-10-19T01:00:00Z"));
        assert_eq!(block.end_time, at("2026-10-19T06:00:00Z"));
        assert_eq!(block.elapsed_seconds, 3 * 3600 + 10 * 60 + 20);
        assert_eq!(block.remaining_seconds, 3600 + 49 * 60 + 40);
        // The burn rate runs from the first request, not from T0.
        let active_minutes = (now - first.requested_at).num_seconds() as f64 / 60.0;
        let expected = block.usage.total_tokens as f64 / active_minutes;
        assert!((block.burn_rate.tokens_per_minute - expected).abs() < 1e-9);

        let blocks = build_blocks(&[&first, &late, &after, &evening]);
        let starts: Vec<_> = blocks.iter().map(|b| (b.start(), b.acc.count)).collect();
        assert_eq!(
            starts,
            vec![
                (at("2026-10-19T01:00:00Z"), 2),
                (at("2026-10-19T11:30:00Z"), 1),
                (at("2026-10-20T00:40:00Z"), 1),
            ]
        );
    }

    #[test]
    fn active_block_reports_countdown_and_burn_rate() {
        let entries = [
            entry("a", "2026-10-17T10:00:00Z", "p", 400),
            entry("b", "2026-10-17T10:08:00Z", "p", 200),
        ];
        let data = compute(&entries, &HashMap::new(), at("2026-10-17T10:10:00Z"), &Utc);
        let block = data.active_block.expect("block still open");
        assert_eq!(block.end_time, at("2026-10-17T15:00:00Z"));
        assert_eq!(block.elapsed_seconds, 600);
        assert_eq!(block.remaining_seconds, 4 * 3600 + 50 * 60);
        assert!((block.burn_rate.tokens_per_minute - 60.0).abs() < 1e-9);
        assert!((block.burn_rate.cost_per_hour - 3.6).abs() < 1e-9);
        // No block has ended yet, so the burn rate lasts until the reset.
        assert_eq!(block.projected.total_tokens, 600 + 60 * 290);
        assert_eq!(block.bucket_minutes, 10);
        let timeline: Vec<_> = block
            .timeline
            .iter()
            .map(|b| (b.start, b.usage.total_tokens))
            .collect();
        assert_eq!(
            timeline,
            vec![
                (at("2026-10-17T10:00:00Z"), 600),
                (at("2026-10-17T10:10:00Z"), 0)
            ]
        );

        let later = compute(&entries, &HashMap::new(), at("2026-10-17T15:00:01Z"), &Utc);
        assert!(later.active_block.is_none());
    }

    #[test]
    fn the_current_accounts_chain_skips_every_span_of_another() {
        let mut history = History::default();
        // a's window of 01:00 ended before b signed in (07:00 to 08:00, and
        // again 09:00 to 09:30).
        for (key, ts) in [
            ("a1", "2026-10-17T01:00:00Z"),
            ("b1", "2026-10-17T07:10:00Z"),
            ("a2", "2026-10-17T08:30:00Z"),
            ("b2", "2026-10-17T09:10:00Z"),
            ("a3", "2026-10-17T09:40:00Z"),
            ("a4", "2026-10-17T10:00:00Z"),
        ] {
            history.insert(Arc::new(entry(key, ts, "p", 100)), &Utc);
        }
        history.set_foreign(vec![
            (at("2026-10-17T07:00:00Z"), at("2026-10-17T08:00:00Z")),
            (at("2026-10-17T09:00:00Z"), at("2026-10-17T09:30:00Z")),
        ]);
        let data = history.snapshot(&HashMap::new(), at("2026-10-17T10:30:00Z"), &Utc);
        let block = data.active_block.unwrap();
        assert_eq!(
            (block.start_time, block.end_time, block.usage.message_count),
            (at("2026-10-17T08:30:00Z"), at("2026-10-17T13:30:00Z"), 3)
        );
        let spend = history.quota_spend_since(DateTime::<Utc>::MIN_UTC);
        assert_eq!(spend.len(), 4);
        // The machine's chain still has b's request open its block.
        assert_eq!(
            history.ended_blocks(at("2026-10-17T12:10:01Z"))[1].start,
            at("2026-10-17T07:10:00Z")
        );
    }

    #[test]
    fn another_accounts_requests_open_no_block_and_spend_nothing() {
        let mut history = History::default();
        // Account a from 08:05, b signed in at 10:00.
        for (key, ts) in [
            ("a1", "2026-10-17T08:05:00Z"),
            ("a2", "2026-10-17T09:30:00Z"),
            ("b1", "2026-10-17T10:03:00Z"),
            ("b2", "2026-10-17T10:30:00Z"),
        ] {
            history.insert(Arc::new(entry(key, ts, "p", 100)), &Utc);
        }
        let block = |history: &mut History, now: &str| {
            let data = history.snapshot(&HashMap::new(), at(now), &Utc);
            let block = data.active_block?;
            Some((block.start_time, block.end_time, block.usage.message_count))
        };
        let spend = |history: &History| -> Vec<DateTime<Utc>> {
            let points = history.quota_spend_since(DateTime::<Utc>::MIN_UTC);
            points.into_iter().map(|(at, _)| at).collect()
        };
        assert_eq!(
            block(&mut history, "2026-10-17T11:00:00Z"),
            Some((at("2026-10-17T08:00:00Z"), at("2026-10-17T13:00:00Z"), 4))
        );

        // As b: its window opened with its first request.
        let switch = at("2026-10-17T10:00:00Z");
        history.set_foreign(vec![(DateTime::<Utc>::MIN_UTC, switch)]);
        assert_eq!(
            block(&mut history, "2026-10-17T11:00:00Z"),
            Some((switch, at("2026-10-17T15:00:00Z"), 2))
        );
        assert_eq!(
            history.last_block(),
            Some((switch, at("2026-10-17T15:00:00Z")))
        );
        assert_eq!(
            spend(&history),
            [at("2026-10-17T10:03:00Z"), at("2026-10-17T10:30:00Z")]
        );
        let (projects, _) = history.quota_spend_by_source(DateTime::<Utc>::MIN_UTC);
        assert!((projects["p"] - 0.2).abs() < 1e-9);
        // The machine's own chain, which the archive keeps, is untouched.
        assert_eq!(history.ended_blocks(at("2026-10-17T13:00:01Z")).len(), 1);

        // Requests that come in later join b's block.
        history.insert(
            Arc::new(entry("b3", "2026-10-17T10:50:00Z", "p", 100)),
            &Utc,
        );
        assert_eq!(
            block(&mut history, "2026-10-17T11:00:00Z"),
            Some((switch, at("2026-10-17T15:00:00Z"), 3))
        );

        // As a, signed in again at 11:00 after b: a's own window from 08:00
        // is still open, with a's requests only.
        history.set_foreign(vec![(switch, at("2026-10-17T11:00:00Z"))]);
        assert_eq!(
            block(&mut history, "2026-10-17T11:30:00Z"),
            Some((at("2026-10-17T08:00:00Z"), at("2026-10-17T13:00:00Z"), 2))
        );
        // Past its end, a has no open block until its next request.
        assert_eq!(block(&mut history, "2026-10-17T13:00:01Z"), None);
        assert_eq!(
            history.last_block(),
            Some((at("2026-10-17T08:00:00Z"), at("2026-10-17T13:00:00Z")))
        );

        // Signed in to b before any request of its own: no block yet.
        let mut fresh = History::default();
        fresh.insert(
            Arc::new(entry("a1", "2026-10-17T09:00:00Z", "p", 100)),
            &Utc,
        );
        fresh.set_foreign(vec![(DateTime::<Utc>::MIN_UTC, switch)]);
        assert_eq!(block(&mut fresh, "2026-10-17T10:05:00Z"), None);
        assert_eq!(fresh.last_block(), None);

        // Without spans, everything counts again.
        history.set_foreign(Vec::new());
        assert_eq!(
            block(&mut history, "2026-10-17T11:00:00Z"),
            Some((at("2026-10-17T08:00:00Z"), at("2026-10-17T13:00:00Z"), 5))
        );
        assert_eq!(spend(&history).len(), 5);
    }

    #[test]
    fn projection_ignores_past_blocks_overlapping_the_current_accounts_block() {
        let entries = [
            entry("a", "2026-10-17T08:00:00Z", "p", 100),
            entry("b1", "2026-10-17T10:00:00Z", "p", 100),
            entry("b2", "2026-10-17T12:00:00Z", "p", 200),
        ];
        let mut history = History::default();
        for entry in &entries {
            history.insert(Arc::new(entry.clone()), &Utc);
        }
        history.set_foreign(vec![(DateTime::<Utc>::MIN_UTC, entries[1].requested_at)]);
        let now = at("2026-10-17T13:30:00Z");
        let block = history
            .snapshot(&HashMap::new(), now, &Utc)
            .active_block
            .unwrap();
        assert_eq!(history.blocks.len(), 1);
        assert!(history.blocks[0].end() > block.start_time);
        assert!(now > history.blocks[0].end());
        assert_eq!(block.usage.total_tokens, 300);
        let expected = compute(&entries[1..], &HashMap::new(), now, &Utc)
            .active_block
            .unwrap();
        assert_eq!(
            block.projected.total_tokens,
            expected.projected.total_tokens
        );
        assert_eq!(block.projected.total_tokens, 429);
        assert!((block.projected.cost_usd - expected.projected.cost_usd).abs() < 1e-9);
    }

    #[test]
    fn projection_adds_the_median_rest_of_past_blocks() {
        let projected = |entries: &[UsageEntry], now: &str| {
            let now = at(now);
            let mut history = History::default();
            for entry in entries {
                history.insert(Arc::new(entry.clone()), &Utc);
            }
            let live = history.snapshot(&HashMap::new(), now, &Utc).active_block;
            let full = compute(entries, &HashMap::new(), now, &Utc).active_block;
            let (live, full) = (live.unwrap().projected, full.unwrap().projected);
            assert_eq!(live.total_tokens, full.total_tokens);
            assert!((live.cost_usd - full.cost_usd).abs() < 1e-9);
            (full.total_tokens, full.cost_usd)
        };
        let mut entries = vec![
            // After 30 minutes these blocks went on to use 300, 1000 and 0.
            entry("a1", "2026-10-17T00:00:00Z", "p", 100),
            entry("a2", "2026-10-17T00:30:00Z", "p", 200),
            entry("a3", "2026-10-17T02:00:00Z", "p", 300),
            entry("b1", "2026-10-17T06:00:00Z", "p", 100),
            entry("b2", "2026-10-17T07:00:00Z", "p", 1000),
            entry("c1", "2026-10-17T12:00:00Z", "p", 100),
            // The open block, 30 minutes in.
            entry("d1", "2026-10-17T18:00:00Z", "p", 500),
            entry("d2", "2026-10-17T18:25:00Z", "p", 100),
        ];
        let (tokens, cost) = projected(&entries, "2026-10-17T18:30:00Z");
        assert_eq!(tokens, 600 + 300);
        assert!((cost - 0.9).abs() < 1e-9);
        // None of them used anything after 2 hours.
        assert_eq!(projected(&entries, "2026-10-17T20:00:00Z").0, 600);

        // Even counts take the midpoint.
        entries.retain(|entry| !entry.dedup_key.starts_with('c'));
        assert_eq!(projected(&entries, "2026-10-17T18:30:00Z").0, 600 + 650);

        // Only the latest 20 ended blocks count: 10 that used 1000 more after
        // 30 minutes and 10 that used nothing, not an 11th older busy one.
        let entries: Vec<_> = (0..21)
            .flat_map(|day| {
                let date = format!("2026-09-{:02}", day + 1);
                let mut block = vec![entry(
                    &format!("{day}a"),
                    &format!("{date}T00:00:00Z"),
                    "p",
                    1,
                )];
                if day < 11 {
                    block.push(entry(
                        &format!("{day}b"),
                        &format!("{date}T01:00:00Z"),
                        "p",
                        1000,
                    ));
                }
                block
            })
            .chain([entry("open", "2026-10-17T18:00:00Z", "p", 1)])
            .collect();
        assert_eq!(projected(&entries, "2026-10-17T18:30:00Z").0, 1 + 500);
    }

    #[test]
    fn peak_block_is_the_busiest_block_that_has_ended() {
        let entries = [
            entry("a", "2026-10-17T02:00:00Z", "p", 100),
            entry("b", "2026-10-17T08:30:00Z", "p", 200),
            entry("c", "2026-10-17T09:00:00Z", "p", 100),
            entry("d", "2026-10-17T20:00:00Z", "p", 900),
        ];
        let meta = HashMap::new();
        assert!(
            compute(&entries[..1], &meta, at("2026-10-17T03:00:00Z"), &Utc)
                .peak_block
                .is_none()
        );

        // The open block outweighs the record but is not part of it yet.
        let data = compute(&entries, &meta, at("2026-10-17T21:00:00Z"), &Utc);
        let peak = data.peak_block.expect("ended blocks");
        assert_eq!(peak.start_time, at("2026-10-17T08:30:00Z"));
        assert_eq!(peak.end_time, at("2026-10-17T13:30:00Z"));
        assert_eq!(peak.usage.total_tokens, 300);

        let later = compute(&entries, &meta, at("2026-10-18T01:00:01Z"), &Utc);
        assert_eq!(
            later.peak_block.expect("ended blocks").usage.total_tokens,
            900
        );
    }

    #[test]
    fn yesterday_is_compared_up_to_the_same_time_of_day() {
        // At UTC+8 it is 06:38 on Oct 20, which began at 16:00 UTC the day before.
        let east = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let now = at("2026-10-19T22:38:00Z");
        let entries = [
            entry("day-before", "2026-10-18T15:59:00Z", "p", 1),
            entry("early", "2026-10-18T17:00:00Z", "p", 2),
            entry("same-time", "2026-10-18T22:38:00Z", "p", 4),
            entry("later", "2026-10-18T22:39:00Z", "p", 8),
            entry("today", "2026-10-19T17:00:00Z", "p", 16),
        ];
        let data = compute(&entries, &HashMap::new(), now, &east);
        assert_eq!(data.today.total_tokens, 16);
        assert_eq!(data.yesterday_so_far.total_tokens, 6);
        let mut history = History::default();
        for e in &entries {
            history.insert(Arc::new(e.clone()), &east);
        }
        let snapshot = history.snapshot(&HashMap::new(), now, &east);
        assert_eq!(snapshot.yesterday_so_far.total_tokens, 6);
    }

    #[test]
    fn rolls_up_today_daily_history_and_projects() {
        let entries = [
            entry("a", "2026-10-14T23:00:00Z", "small", 5),
            entry("b", "2026-10-17T01:00:00Z", "big", 100),
            entry("c", "2026-10-17T02:00:00Z", "small", 7),
            entry("c", "2026-10-17T02:00:00Z", "small", 7),
        ];
        let mut meta = HashMap::new();
        meta.insert(
            Arc::from("big"),
            ProjectMeta {
                name: "cc-deskmeter".to_owned(),
                path: Some("/work/cc-deskmeter".to_owned()),
            },
        );
        let data = compute(&entries, &meta, at("2026-10-18T12:00:00Z"), &Utc);

        assert_eq!(data.total.total_tokens, 112);
        assert_eq!(data.total.message_count, 3);
        assert_eq!(data.today.total_tokens, 0);
        let days: Vec<_> = data
            .daily
            .iter()
            .map(|d| (d.date.as_str(), d.usage.total_tokens))
            .collect();
        assert_eq!(
            days,
            vec![
                ("2026-10-14", 5),
                ("2026-10-15", 0),
                ("2026-10-16", 0),
                ("2026-10-17", 107),
                ("2026-10-18", 0)
            ]
        );
        assert_eq!(data.first_day.as_deref(), Some("2026-10-14"));
        let months: Vec<_> = data
            .monthly
            .iter()
            .map(|m| (m.month.as_str(), m.usage.total_tokens))
            .collect();
        assert_eq!(months, [("2026-10", 112)]);
        assert_eq!(data.projects[0].name, "cc-deskmeter");
        assert_eq!(data.projects[0].path.as_deref(), Some("/work/cc-deskmeter"));
        assert_eq!(data.projects[1].id, "small");
        assert_eq!(data.projects[1].usage.total_tokens, 12);
        assert_eq!(data.projects[1].session_count, 1);
    }

    #[test]
    fn quota_spend_is_summed_per_project_and_model_after_a_moment() {
        let mut sonnet = entry("c", "2026-10-18T10:30:00Z", "q", 4000);
        sonnet.model = "claude-sonnet-5-5".into();
        let mut history = History::default();
        for e in [
            entry("a", "2026-10-18T09:00:00Z", "p", 1000),
            entry("b", "2026-10-18T10:10:00Z", "p", 2000),
            sonnet,
        ] {
            history.insert(Arc::new(e), &Utc);
        }
        let (projects, models) = history.quota_spend_by_source(at("2026-10-18T10:00:00Z"));
        assert_eq!(
            (projects.len(), projects["p"], projects["q"]),
            (2, 2.0, 4.0)
        );
        assert_eq!(
            (models["claude-opus-5-5"], models["claude-sonnet-5-5"]),
            (2.0, 4.0)
        );
    }

    #[test]
    fn archived_quota_spend_excludes_foreign_days_and_prorates_partial_days() {
        use super::super::archive::merge_days;

        let start = at("2026-10-18T00:00:00Z");
        let end = at("2026-10-19T00:00:00Z");
        let now = end + TimeDelta::hours(12);
        let mut live = History::default();
        live.insert(
            Arc::new(entry("gone", "2026-10-18T08:00:00Z", "p", 1000)),
            &Utc,
        );
        let archive = Archive::from_parts(merge_days(&[], live.entries(), now, &Utc), Vec::new());
        let mut history = History::default();
        history.set_archive(&archive, &Pricing::default());
        for (spans, expected) in [
            (vec![(DateTime::<Utc>::MIN_UTC, end)], 0.0),
            (
                vec![(DateTime::<Utc>::MIN_UTC, start + TimeDelta::hours(12))],
                0.5,
            ),
            (
                vec![
                    (start, start + TimeDelta::hours(6)),
                    (end - TimeDelta::hours(6), end),
                ],
                0.5,
            ),
            (vec![(end, now)], 1.0),
            (Vec::new(), 1.0),
        ] {
            history.set_foreign(spans);
            let data = history.snapshot(&HashMap::new(), now, &Utc);
            assert_eq!(data.total.message_count, 1);
            assert_eq!(history.filled_responses(), 1);
            let (projects, models) = history.quota_spend_by_source(start);
            if expected == 0.0 {
                assert!(projects.is_empty() && models.is_empty());
            } else {
                assert!((projects["p"] - expected).abs() < 1e-9);
                assert!((models["claude-opus-5-5"] - expected).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn the_archive_restores_deleted_sessions_without_counting_anything_twice() {
        use super::super::archive::{merge_blocks, merge_days};

        let now = at("2026-10-20T12:00:00Z");
        let models = ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"];
        let mut entries = Vec::new();
        let mut seed = 11u64;
        for i in 0..600 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let mut e = entry(
                &format!("m{i}"),
                "2026-10-13T00:00:00Z",
                &format!("p{}", (seed >> 33) % 4),
                (seed >> 24) % 900 + 1,
            );
            // A week up to now, with a quiet stretch so blocks reopen.
            e.timestamp += TimeDelta::seconds(((seed >> 7) % (7 * 86_400 + 40_000)) as i64);
            if e.timestamp > now {
                continue;
            }
            e.requested_at = e.timestamp - TimeDelta::seconds(((seed >> 40) % 90) as i64);
            e.model = models[((seed >> 50) % 3) as usize].into();
            e.tokens.cache_read = (seed >> 13) % 5000;
            e.cost_usd = (seed % 100) as f64 / 1024.0;
            e.quota_usd = e.cost_usd * 0.75;
            // Sessions of a few hours, kept apart by project.
            let day = (e.timestamp - at("2026-10-13T00:00:00Z")).num_hours() / 6;
            e.session_id = format!("{}-{day}", e.project);
            entries.push(e);
        }
        // A project whose only session will be deleted.
        entries.push(entry("gone-1", "2026-10-15T08:00:00Z", "gone", 700));
        entries.push(entry("gone-2", "2026-10-15T09:00:00Z", "gone", 300));

        let meta = HashMap::new();
        let mut full = History::default();
        for e in &entries {
            full.insert(Arc::new(e.clone()), &Utc);
        }
        let expected = full.snapshot(&meta, now, &Utc);
        let archive = Archive::from_parts(
            merge_days(&[], full.entries(), now, &Utc),
            merge_blocks(&[], full.ended_blocks(now)),
        );
        assert!(!archive.days().is_empty() && !archive.blocks().is_empty());
        let pricing = Pricing::default();

        // Every transcript is still there: the archive adds nothing.
        let mut intact = History::default();
        intact.set_archive(&archive, &pricing);
        for e in &entries {
            intact.insert(Arc::new(e.clone()), &Utc);
        }
        assert_payload(&intact.snapshot(&meta, now, &Utc), &expected);
        assert_eq!(intact.filled_responses(), 0);

        // Claude Code deletes whole sessions that ended before today.
        let today = at("2026-10-20T00:00:00Z");
        let mut last_seen: HashMap<&str, DateTime<Utc>> = HashMap::new();
        for e in &entries {
            let last = last_seen.entry(&e.session_id).or_insert(e.timestamp);
            *last = (*last).max(e.timestamp);
        }
        let deleted = |e: &UsageEntry| {
            let n: u64 = e.session_id.bytes().map(u64::from).sum();
            last_seen[e.session_id.as_str()] < today
                && (n.is_multiple_of(3) || e.project.as_ref() == "gone")
        };
        let kept: Vec<_> = entries.iter().filter(|e| !deleted(e)).collect();
        assert!(kept.len() < entries.len() - 50, "deletes a fair share");
        let mut partial = History::default();
        partial.set_archive(&archive, &pricing);
        for e in &kept {
            partial.insert(Arc::new((*e).clone()), &Utc);
        }
        let mut restored = partial.snapshot(&meta, now, &Utc);
        // The projection learns from the responses still on disk: archived
        // blocks keep totals, not when within them the usage arrived.
        let open = restored.active_block.as_mut().unwrap();
        open.projected = expected.active_block.as_ref().unwrap().projected.clone();
        // Nor when within its day: yesterday so far keeps what is on disk.
        let so_far = |data: &UsageData| data.yesterday_so_far.message_count;
        assert!(so_far(&restored) < so_far(&expected) && so_far(&restored) > 0);
        restored.yesterday_so_far = expected.yesterday_so_far.clone();
        assert_payload(&restored, &expected);
        assert!(!partial.overrides.is_empty() && partial.overrides.len() < archive.blocks().len());
        let mut bare = History::default();
        for e in &kept {
            bare.insert(Arc::new((*e).clone()), &Utc);
        }
        let bare = bare.snapshot(&meta, now, &Utc);
        assert!(bare.total.message_count < expected.total.message_count);
        assert!(
            bare.daily.len() < expected.daily.len()
                || bare.projects.len() < expected.projects.len()
        );
        assert_eq!(
            partial.filled_responses(),
            (entries.len() - kept.len()) as u64
        );
        assert!(restored.projects.iter().any(|p| p.id == "gone"));
        // Archived again, nothing changes.
        assert_eq!(
            merge_days(archive.days(), partial.entries(), now, &Utc),
            archive.days()
        );
        assert_eq!(
            merge_blocks(archive.blocks(), partial.ended_blocks(now)),
            archive.blocks()
        );
        // Attribution sees the archived whole days.
        let since = at("2026-10-15T00:00:00Z");
        let (projects, _) = partial.quota_spend_by_source(since);
        let (expected_projects, _) = full.quota_spend_by_source(since);
        for (project, usd) in expected_projects {
            assert!((projects[&project] - usd).abs() < 1e-9, "{project}");
        }

        // In another zone the days regroup; the totals stay.
        let east = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        let moved = partial.snapshot(&meta, now, &east);
        let expected = full.snapshot(&meta, now, &east);
        assert_eq!(moved.total.message_count, expected.total.message_count);
        assert_eq!(moved.total.total_tokens, expected.total.total_tokens);
        assert!((moved.total.cost_usd - expected.total.cost_usd).abs() < 1e-9);
    }

    #[test]
    fn partial_sessions_remain_unique_after_rearchiving_and_restart() {
        use super::super::archive::{merge_blocks, merge_days};
        let now = at("2026-10-20T12:00:00Z");
        let early = entry("first", "2026-10-17T10:00:00Z", "p", 100);
        let later = entry("later", "2026-10-18T10:00:00Z", "p", 200);
        let today = entry("today", "2026-10-20T10:00:00Z", "p", 300);
        let mut full = History::default();
        for e in [&early, &later, &today] {
            full.insert(Arc::new(e.clone()), &Utc);
        }
        full.snapshot(&HashMap::new(), now, &Utc);
        let archive = Archive::from_parts(
            merge_days(&[], full.entries(), now, &Utc),
            merge_blocks(&[], full.ended_blocks(now)),
        );
        for retained in [vec![&later, &today], vec![&today]] {
            let mut partial = History::default();
            partial.set_archive(&archive, &Pricing::default());
            for e in retained {
                partial.insert(Arc::new(e.clone()), &Utc);
            }
            let data = partial.snapshot(&HashMap::new(), now, &Utc);
            assert_eq!(data.projects[0].session_count, 1);
            assert_eq!(data.total.input_tokens, 600);
            let merged = merge_days(archive.days(), partial.entries(), now, &Utc);
            assert_eq!(merged, archive.days());
            let saved = Archive::from_parts(merged, archive.blocks().to_vec());
            let mut restarted = History::default();
            restarted.set_archive(&saved, &Pricing::default());
            // Even a session with no live entries yet must stop counting as
            // restored when its next response arrives after the archive's end.
            assert_eq!(
                restarted.snapshot(&HashMap::new(), now, &Utc).projects[0].session_count,
                1
            );
            restarted.insert(Arc::new(today.clone()), &Utc);
            assert_eq!(
                restarted.snapshot(&HashMap::new(), now, &Utc).projects[0].session_count,
                1
            );
            let mut other = entry("new", "2026-10-20T11:00:00Z", "p", 10);
            other.session_id = "new-session".into();
            restarted.insert(Arc::new(other), &Utc);
            assert_eq!(
                restarted.snapshot(&HashMap::new(), now, &Utc).projects[0].session_count,
                2
            );
        }
    }

    #[test]
    fn archive_dates_stay_fixed_across_timezone_changes_and_source_deletion() {
        use super::super::archive::merge_days;
        let now = at("2026-10-20T12:00:00Z");
        let original = entry("first", "2026-10-17T23:30:00Z", "p", 100);
        let mut full = History::default();
        full.insert(Arc::new(original.clone()), &Utc);
        full.snapshot(&HashMap::new(), now, &Utc);
        let archive = Archive::from_parts(merge_days(&[], full.entries(), now, &Utc), Vec::new());
        let east = FixedOffset::east_opt(8 * 3600).unwrap();
        // Installing an archive must also regroup entries already indexed in
        // this timezone, even if an append follows before the next snapshot.
        let mut live = History::default();
        live.insert(Arc::new(original.clone()), &east);
        live.snapshot(&HashMap::new(), now, &east);
        live.set_archive(&archive, &Pricing::default());
        let added = entry("second", "2026-10-17T23:40:00Z", "p", 200);
        live.insert(Arc::new(added), &east);
        let day_totals = |data: UsageData| {
            data.daily
                .into_iter()
                .filter(|day| day.usage.total_tokens > 0)
                .map(|day| (day.date, day.usage.total_tokens))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            day_totals(live.snapshot(&HashMap::new(), now, &east)),
            vec![("2026-10-17".into(), 300)]
        );
        let saved = Archive::from_parts(
            merge_days(archive.days(), live.entries(), now, &east),
            Vec::new(),
        );
        let mut gone = History::default();
        gone.set_archive(&saved, &Pricing::default());
        assert_eq!(
            day_totals(gone.snapshot(&HashMap::new(), now, &east)),
            vec![("2026-10-17".into(), 300)]
        );
        assert_eq!(
            day_totals(live.snapshot(&HashMap::new(), now, &Utc)),
            vec![("2026-10-17".into(), 300)]
        );

        // Replacing a winner removes it from the same archived date it was
        // added to; records outside archived spans still use the current zone.
        let mut replacement = original;
        replacement.timestamp = at("2026-10-18T23:30:00Z");
        replacement.requested_at = replacement.timestamp;
        live.insert(Arc::new(replacement), &east);
        assert_eq!(
            day_totals(live.snapshot(&HashMap::new(), now, &east)),
            vec![("2026-10-17".into(), 200), ("2026-10-19".into(), 100)]
        );
    }

    #[test]
    fn cache_hit_ratio_uses_prompt_side_tokens() {
        let mut e = entry("a", "2026-10-17T10:00:00Z", "p", 10);
        e.tokens = TokenCounts {
            input: 10,
            output: 500,
            cache_creation: 90,
            cache_read: 900,
        };
        let data = compute([&e], &HashMap::new(), at("2026-10-17T10:00:00Z"), &Utc);
        assert!((data.total.cache_hit_ratio - 0.9).abs() < 1e-9);
    }
}
