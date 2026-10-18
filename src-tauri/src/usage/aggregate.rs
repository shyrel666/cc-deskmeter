//! Deduplication, 5-hour rolling blocks, and the rollups sent to the frontend.

#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use chrono::{DateTime, Datelike, DurationRound, NaiveDate, Offset, TimeDelta, TimeZone, Utc};

use super::types::{
    ActiveBlock, BlockBucket, BurnRate, DailyUsage, ModelUsage, PastBlock, ProjectUsage,
    Projection, TokenCounts, UsageData, UsageEntry, UsageStats,
};

const BLOCK_HOURS: i64 = 5;
/// Resolution of the active block's timeline (30 buckets per block).
const BUCKET_MINUTES: i64 = 10;
const BUCKET_COUNT: usize = (BLOCK_HOURS * 60 / BUCKET_MINUTES) as usize;

fn block_duration() -> TimeDelta {
    TimeDelta::hours(BLOCK_HOURS)
}

/// Display info for a project folder, resolved from the `cwd` in its logs.
#[derive(Debug, Clone)]
pub struct ProjectMeta {
    pub name: String,
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
}

impl Block {
    /// Claude starts its own window at the minute the first request was sent,
    /// so its reset time is a whole minute too.
    pub fn start(&self) -> DateTime<Utc> {
        self.opened
            .duration_trunc(TimeDelta::minutes(1))
            .unwrap_or(self.opened)
    }

    pub fn end(&self) -> DateTime<Utc> {
        self.start() + block_duration()
    }
}

/// CLAUDE.md §3.1: the first request opens a block at T0 (its minute); every
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
            });
        }
    }
}

/// Status of `block` at `now`, or `None` once the block has ended. `buckets`
/// contain its cached usage across the entire five-hour window.
///
/// Burn rate is measured over the time elapsed since the block opened (at
/// least one minute), so it decays while idle and the projection assumes the
/// current pace continues until the block ends.
fn active_block(
    block: &Block,
    buckets: &[Accum; BUCKET_COUNT],
    now: DateTime<Utc>,
) -> Option<ActiveBlock> {
    let (start, end) = (block.start(), block.end());
    if now > end {
        return None;
    }
    let elapsed_seconds = (now - start).num_seconds().max(0);
    let remaining_seconds = (end - now).num_seconds().max(0);
    let elapsed_minutes = (elapsed_seconds as f64 / 60.0).max(1.0);
    let remaining_minutes = remaining_seconds as f64 / 60.0;

    let usage = block.acc.stats();
    let tokens_per_minute = usage.total_tokens as f64 / elapsed_minutes;
    let cost_per_hour = usage.cost_usd / elapsed_minutes * 60.0;

    Some(ActiveBlock {
        start_time: start,
        end_time: end,
        last_activity: block.last_activity,
        elapsed_seconds,
        remaining_seconds,
        projected: Projection {
            total_tokens: usage.total_tokens
                + (tokens_per_minute * remaining_minutes).round() as u64,
            cost_usd: usage.cost_usd + cost_per_hour * remaining_minutes / 60.0,
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
/// history) are taken in `tz`. `data_dir` and `file_count` are left for the
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

    let mut total = Accum::default();
    let mut today_acc = Accum::default();
    let mut daily: BTreeMap<NaiveDate, Accum> = BTreeMap::new();
    let mut by_project: HashMap<&Arc<str>, ProjectAccum<'_>> = HashMap::new();
    let mut by_model: HashMap<&str, (Accum, bool)> = HashMap::new();

    for &entry in &sorted {
        let day = entry.timestamp.with_timezone(tz).date_naive();
        total.add(entry);
        if day == today {
            today_acc.add(entry);
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
    let active_block = blocks.last().and_then(|block| {
        // The last block holds every message from its opening request onwards.
        let first = by_request.partition_point(|entry| entry.requested_at < block.opened);
        active_block(
            block,
            &block_buckets(block.start(), by_request[first..].iter().copied()),
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

    UsageData {
        snapshot_id: 0,
        generated_at: now,
        data_dir: None,
        file_count: 0,
        read_errors: Vec::new(),
        today: today_acc.stats(),
        active_block,
        peak_block: peak_block(&blocks, now),
        daily: daily_series(&daily, today),
        projects,
        models,
        total: total.stats(),
    }
}

/// One row per day from the first recorded day through today, zero-filled.
fn daily_series(daily: &BTreeMap<NaiveDate, Accum>, today: NaiveDate) -> Vec<DailyUsage> {
    let (Some((&first, _)), Some((&last, _))) = (daily.first_key_value(), daily.last_key_value())
    else {
        return Vec::new();
    };
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

#[derive(Default)]
struct ProjectTotals {
    acc: Accum,
    sessions: HashMap<String, usize>,
    activity: BTreeMap<DateTime<Utc>, usize>,
}

/// Incremental winners and rollups. File deletion/rewrite rebuilds this index;
/// ordinary appends only replace the affected message, day, project and model.
/// The ordered index lets late records rebuild only the affected block suffix.
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
        let mut first_changed = entry.requested_at;
        if let Some((old, day)) = self.entries.remove(&entry.dedup_key) {
            first_changed = first_changed.min(old.requested_at);
            self.ordered
                .remove(&(old.requested_at, Arc::clone(&old.dedup_key)));
            self.remove_rollup(&old, day);
        }
        let day = entry.timestamp.with_timezone(tz).date_naive();
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
        if self.zone == Some(fingerprint) {
            return;
        }
        // DST or a timezone switch can change calendar grouping, while UTC
        // ordering, block boundaries and non-calendar rollups remain valid.
        self.daily.clear();
        for (entry, day) in self.entries.values_mut() {
            *day = entry.timestamp.with_timezone(tz).date_naive();
            self.daily.entry(*day).or_default().add(entry);
        }
        self.zone = Some(fingerprint);
    }

    fn refresh_blocks(&mut self) {
        let Some(changed) = self.blocks_dirty_from.take() else {
            return;
        };
        let keep = self.blocks.partition_point(|block| block.end() < changed);
        let from = self
            .blocks
            .get(keep)
            .map_or(changed, |block| block.opened.min(changed));
        self.blocks.truncate(keep);
        for entry in self
            .ordered
            .range((from, Arc::from(""))..)
            .map(|(_, entry)| entry)
        {
            push_block(&mut self.blocks, entry);
        }
        self.buckets = self
            .blocks
            .last()
            .map_or([Accum::default(); BUCKET_COUNT], |block| {
                block_buckets(
                    block.start(),
                    self.ordered
                        .range((block.opened, Arc::from(""))..)
                        .map(|(_, e)| e.as_ref()),
                )
            });
    }

    pub fn snapshot<Tz: TimeZone>(
        &mut self,
        meta: &HashMap<Arc<str>, ProjectMeta>,
        now: DateTime<Utc>,
        tz: &Tz,
    ) -> UsageData {
        self.check_timezone(now, tz);
        self.refresh_blocks();
        let today = now.with_timezone(tz).date_naive();
        let mut projects: Vec<_> = self
            .projects
            .iter()
            .map(|(id, p)| {
                let info = meta.get(id);
                ProjectUsage {
                    id: id.to_string(),
                    name: info.map_or_else(|| id.to_string(), |m| m.name.clone()),
                    path: info.and_then(|m| m.path.clone()),
                    session_count: p.sessions.len(),
                    last_activity: *p.activity.last_key_value().expect("nonempty project").0,
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
        let mut models: Vec<_> = self
            .models
            .iter()
            .map(|(model, (acc, known))| ModelUsage {
                model: model.clone(),
                pricing_known: *known,
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
        UsageData {
            snapshot_id: 0,
            generated_at: now,
            data_dir: None,
            file_count: 0,
            read_errors: Vec::new(),
            today: self.daily.get(&today).copied().unwrap_or_default().stats(),
            active_block: self
                .blocks
                .last()
                .and_then(|block| active_block(block, &self.buckets, now)),
            peak_block: peak_block(&self.blocks, now),
            daily: daily_series(&self.daily, today),
            projects,
            models,
            total: self.total.stats(),
        }
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
            cost_usd: input as f64 / 1000.0,
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
        let days = |data: &UsageData| -> Vec<String> {
            data.daily
                .iter()
                .filter(|day| day.usage.message_count > 0)
                .map(|day| day.date.clone())
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
        assert_eq!(days(&data), ["2026-07-14"]);
        // Back to Denver: October's offset is the same, July's is not.
        PHOENIX.store(false, Ordering::SeqCst);
        assert_eq!(days(&history.snapshot(&meta, october, &Mountain)), ["2026-07-15"]);
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
            for times in [&mut full, &mut idle, &mut append] {
                times.sort_by(f64::total_cmp);
            }
            println!(
                "entries={count} build_ms={build:.1} full_ms={:.3} cached_tick_ms={:.3} append_and_snapshot_ms={:.3}",
                full[2], idle[2], append[2]
            );
        }
    }

    #[test]
    fn blocks_include_the_5h_boundary_and_split_after_it() {
        let entries = [
            entry("a", "2026-10-17T08:13:00Z", "p", 1),
            entry("b", "2026-10-17T12:00:00Z", "p", 1),
            entry("c", "2026-10-17T13:13:00Z", "p", 1), // exactly T0 + 5h
            entry("d", "2026-10-17T13:13:01Z", "p", 1), // first message after T0 + 5h
            entry("e", "2026-10-17T20:00:30Z", "p", 1),
        ];
        let sorted = dedup(&entries);
        let blocks = build_blocks(&sorted);
        let starts: Vec<_> = blocks.iter().map(|b| (b.start(), b.acc.count)).collect();
        assert_eq!(
            starts,
            vec![
                (at("2026-10-17T08:13:00Z"), 3),
                (at("2026-10-17T13:13:00Z"), 1),
                (at("2026-10-17T20:00:00Z"), 1),
            ]
        );
    }

    #[test]
    fn blocks_open_at_the_minute_of_the_first_request() {
        // Prompt sent at 01:51:59, first response logged at 01:52:01.
        let mut first = entry("a", "2026-10-18T01:52:01.772Z", "p", 1);
        first.requested_at = at("2026-10-18T01:51:59.025Z");
        // Sent before the reset but answered after it: still the same block.
        let mut late = entry("b", "2026-10-18T06:51:05Z", "p", 1);
        late.requested_at = at("2026-10-18T06:50:58Z");
        let after = entry("c", "2026-10-18T06:51:01Z", "p", 1);

        let data = compute([&first], &HashMap::new(), at("2026-10-18T04:10:20Z"), &Utc);
        let block = data.active_block.expect("block still open");
        assert_eq!(block.start_time, at("2026-10-18T01:51:00Z"));
        assert_eq!(block.end_time, at("2026-10-18T06:51:00Z"));
        assert_eq!(block.remaining_seconds, 2 * 3600 + 40 * 60 + 40);

        let blocks = build_blocks(&[&first, &late, &after]);
        let starts: Vec<_> = blocks.iter().map(|b| (b.start(), b.acc.count)).collect();
        assert_eq!(
            starts,
            vec![
                (at("2026-10-18T01:51:00Z"), 2),
                (at("2026-10-18T06:51:00Z"), 1),
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
                name: "claude-pulse".to_owned(),
                path: Some("/work/claude-pulse".to_owned()),
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
        assert_eq!(data.projects[0].name, "claude-pulse");
        assert_eq!(data.projects[0].path.as_deref(), Some("/work/claude-pulse"));
        assert_eq!(data.projects[1].id, "small");
        assert_eq!(data.projects[1].usage.total_tokens, 12);
        assert_eq!(data.projects[1].session_count, 1);
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
