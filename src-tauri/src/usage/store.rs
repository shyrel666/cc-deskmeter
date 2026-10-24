//! Incremental reader for `<root>/**/*.jsonl` across every discovered
//! transcript root (see `paths`).
//!
//! Appends check bounded head/tail samples and read only new bytes. Full prefix
//! checks run off the store lock in the background; manual rescans reparse all
//! files. Truncations, replacements and detected rewrites rebuild their index.
//! An optional archive (`open_archive`) keeps aggregates once transcripts go.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use chrono::{DateTime, Local, Utc};
use serde::de::IgnoredAny;
use walkdir::WalkDir;

use super::aggregate::{History, ProjectMeta};
use super::archive::{self, Archive};
use super::parser::{self, ParsedUsage, RecordContext};
use super::paths::Roots;
use super::pricing::{self, Pricing};
use super::quota::{self, LocalBlock, PercentPerUsd, QuotaSource, Sample, Spend};
use super::types::{ArchiveSummary, ReadError, TokenCounts, UsageData, UsageEntry};

/// Parent links kept per transcript. A response's chain reaches back only to
/// the prompt or tool result that sent it (at most 4 records in real logs);
/// an evicted parent falls back to the response's own timestamp.
const MAX_RECORD_LINKS: usize = 1024;

/// Locks the store, recovering from a poisoned mutex (its data stays consistent).
pub fn lock(store: &Mutex<UsageStore>) -> MutexGuard<'_, UsageStore> {
    store.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct UsageStore {
    roots: Roots,
    files: HashMap<PathBuf, FileState>,
    /// Bumped whenever usage, file metadata displayed by the UI or read health changes.
    generation: u64,
    snapshot_id: u64,
    history: History,
    project_meta: HashMap<Arc<str>, ProjectMeta>,
    read_errors: BTreeMap<PathBuf, String>,
    pricing: Pricing,
    /// Optional local price overrides; `None` uses the built-in table only.
    pricing_file: Option<PathBuf>,
    /// Metadata of `pricing_file` at its last completed read, `Some(None)`
    /// while it is absent. `None` makes the next refresh read it.
    pricing_stamp: Option<Option<FileStamp>>,
    pricing_error: Option<String>,
    quota: QuotaSource,
    /// The quota estimate's spend and fit as of the last snapshot.
    quota_fit: Option<QuotaFit>,
    /// Usage and weekly-limit readings kept beyond the transcripts and
    /// Desktop's own log; `None` until `open_archive`.
    archive: Option<Archive>,
    /// `(start of today, end of the latest ended block)` when the archive
    /// last took in the finished days and blocks.
    archived_through: Option<(DateTime<Utc>, Option<DateTime<Utc>>)>,
    /// The archive has not seen Desktop's current samples yet.
    readings_due: bool,
}

impl UsageStore {
    pub fn new(roots: Roots) -> Self {
        Self {
            roots,
            files: HashMap::new(),
            generation: 0,
            snapshot_id: 0,
            history: History::default(),
            project_meta: HashMap::new(),
            read_errors: BTreeMap::new(),
            pricing: Pricing::default(),
            pricing_file: None,
            pricing_stamp: None,
            pricing_error: None,
            quota: QuotaSource::default(),
            quota_fit: None,
            archive: None,
            archived_through: None,
            readings_due: false,
        }
    }

    /// Keeps an aggregates-only archive in `dir` (see `archive`) and merges
    /// it into every snapshot from now on. Cheap before the first refresh,
    /// which then does the initial scan.
    pub fn open_archive(&mut self, dir: PathBuf) {
        // Local prices first, so the archive's costs are checked against them.
        self.reload_pricing();
        let archive = Archive::open(dir, &self.pricing);
        self.history.set_archive(&archive, &self.pricing);
        self.archive = Some(archive);
        self.archived_through = None;
        self.readings_due = true;
        self.project_meta = self.project_meta();
        self.generation += 1;
    }

    /// Reads Claude Desktop's plan usage samples from the newest of `paths` on
    /// every snapshot (cheap when unchanged); none existing leaves the quota out.
    pub fn with_quota_files(mut self, paths: Vec<PathBuf>) -> Self {
        self.quota = QuotaSource::new(paths);
        self
    }

    /// Reads local price overrides from `path`, if it exists, on every full
    /// refresh (so edits apply within one tick) and on every rescan. New
    /// prices reprice what was read already, without reading it again.
    pub fn with_pricing_file(mut self, path: Option<PathBuf>) -> Self {
        self.pricing_file = path;
        self
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots.dirs
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Picks up new lines, new files and deleted files. Returns whether any
    /// usage entry was added or removed, or prices changed.
    pub fn refresh(&mut self) -> bool {
        let repriced = self.reload_pricing();
        self.refresh_inner(false, &HashSet::new()) || repriced
    }

    /// Inspect only notified files/subtrees, including removals. Equal-length
    /// writes are verified even if the writer preserved the modification time.
    pub fn refresh_paths(&mut self, paths: &HashSet<PathBuf>) -> bool {
        self.refresh_inner(false, paths)
    }

    pub fn rescan(&mut self) -> bool {
        let repriced = self.reload_pricing();
        self.refresh_inner(true, &HashSet::new()) || repriced
    }

    /// Re-reads the overrides when the file appeared, changed or went away,
    /// or when the last read failed. Returns whether prices changed, and so
    /// every cost (see `reprice`). An unreadable or invalid file keeps the
    /// last valid prices.
    fn reload_pricing(&mut self) -> bool {
        let Some(path) = self.pricing_file.as_deref() else {
            return false;
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .map(|meta| FileStamp::from(&meta));
        if self.pricing_stamp.as_ref() == Some(&stamp) {
            return false;
        }
        let read = std::fs::read_to_string(path);
        // A read error can pass without touching the metadata (a sharing
        // violation, a permission fixed with chmod), so it is retried on the
        // next refresh. Invalid content waits for the file to change.
        self.pricing_stamp = match &read {
            Err(err) if err.kind() != io::ErrorKind::NotFound => None,
            _ => Some(stamp),
        };
        let loaded = match read {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Pricing::default()),
            Err(err) => Err(err.to_string()),
            Ok(text) => Pricing::parse(&text),
        };
        let (pricing, error) = match loaded {
            Ok(pricing) => (pricing, None),
            Err(err) => (self.pricing.clone(), Some(err)),
        };
        if error != self.pricing_error {
            // Logged once per distinct error, not on every retry.
            if let Some(err) = &error {
                eprintln!("cc-deskmeter: ignoring {}: {err}", path.display());
            }
            self.pricing_error = error;
            self.generation += 1;
        }
        if pricing == self.pricing {
            return false;
        }
        self.pricing = pricing;
        self.reprice();
        true
    }

    /// Recomputes every cost at `self.pricing` from the responses already
    /// read. Nothing is read again, so a transcript that cannot be opened
    /// right now is repriced too, and the archive (repriced from its tokens,
    /// then merged again) never records prices that some costs do not match.
    fn reprice(&mut self) {
        for state in self.files.values_mut() {
            for entry in state.entries.values_mut() {
                let price = price_of(
                    &self.pricing,
                    &entry.model,
                    &entry.tokens,
                    entry.cache_write_1h,
                    entry.fast,
                );
                if !price.matches(entry) {
                    *entry = Arc::new(price.apply(UsageEntry::clone(entry)));
                }
            }
        }
        if let Some(archive) = &mut self.archive {
            archive.reprice(&self.pricing);
        }
        self.rebuild_history();
        self.archived_through = None;
        self.generation += 1;
    }

    /// Indexes every response read so far afresh, merged with the archive.
    fn rebuild_history(&mut self) {
        self.history = History::default();
        if let Some(archive) = &self.archive {
            self.history.set_archive(archive, &self.pricing);
        }
        for entry in self.files.values().flat_map(|state| state.entries.values()) {
            self.history.insert(Arc::clone(entry), &Local);
        }
    }

    /// Capture small, immutable jobs; callers must release the store lock
    /// before doing their potentially large reads. Routine passes audit only
    /// transcripts that grew since their prefix was last verified;
    /// `include_unchanged` audits every transcript.
    pub fn validation_jobs(&self, include_unchanged: bool) -> Vec<ValidationJob> {
        self.files
            .iter()
            .filter(|(_, state)| {
                state.offset > 0 && (include_unchanged || state.verified < state.offset)
            })
            .map(|(path, state)| ValidationJob {
                path: path.clone(),
                offset: state.offset,
                checksum: state.checksum.clone().finalize(),
                incarnation: Arc::clone(&state.incarnation),
            })
            .collect()
    }

    pub fn apply_validation(&mut self, validation: ValidationResult) -> bool {
        let job = validation.job;
        let Some(state) = self.files.get_mut(&job.path) else {
            return false;
        };
        // A rescan/replacement since the job was captured makes it obsolete.
        // Appends keep the incarnation, so a busy file can still be verified.
        if !Arc::ptr_eq(&state.incarnation, &job.incarnation) {
            return false;
        }
        match validation.result {
            Ok(true) => {
                state.verified = state.verified.max(job.offset);
                false
            }
            Ok(false) => self.refresh_inner(true, &HashSet::from([job.path])),
            Err(message) => {
                state.failed = true;
                let changed = self.read_errors.get(&job.path) != Some(&message);
                self.read_errors.insert(job.path, message);
                if changed {
                    self.generation += 1;
                }
                changed
            }
        }
    }

    fn refresh_inner(&mut self, force: bool, paths: &HashSet<PathBuf>) -> bool {
        let roots = self.roots.dirs.clone();
        // The innermost root a path lies under, should one root contain another.
        let root_of = |path: &Path| {
            roots
                .iter()
                .filter(|root| path.starts_with(root))
                .max_by_key(|root| root.components().count())
        };
        let mut candidates: Vec<_> = if paths.is_empty() {
            roots.clone()
        } else {
            paths
                .iter()
                .filter(|p| root_of(p).is_some())
                .cloned()
                .collect()
        };
        candidates.sort();
        let mut scopes: Vec<PathBuf> = Vec::new();
        for candidate in candidates {
            if !scopes.iter().any(|parent| candidate.starts_with(parent)) {
                scopes.push(candidate);
            }
        }
        if scopes.is_empty() {
            return false;
        }
        let affected = |path: &Path| scopes.iter().any(|scope| path.starts_with(scope));
        let previous_errors = self.read_errors.clone();
        self.read_errors.retain(|path, _| !affected(path));
        let mut seen = HashSet::new();
        let mut changed = false;
        let mut rebuild = false;
        let mut complete_scan = true;
        let previous_file_count = self.files.len();

        let walks = scopes.iter().flat_map(|scope| {
            let root = root_of(scope).expect("scopes lie under a root");
            WalkDir::new(scope)
                .into_iter()
                .map(move |item| (root, item))
        });
        for (root, item) in walks {
            let item = match item {
                Ok(item) => item,
                Err(err) => {
                    // Only a successful enumeration can prove that a cached
                    // file was deleted. A missing root is a real deletion.
                    if err
                        .io_error()
                        .is_none_or(|e| e.kind() != io::ErrorKind::NotFound)
                    {
                        complete_scan = false;
                        eprintln!("cc-deskmeter: failed to enumerate transcripts: {err}");
                        self.read_errors
                            .insert(err.path().unwrap_or(root).to_path_buf(), err.to_string());
                    }
                    continue;
                }
            };
            let is_jsonl = item.path().extension().is_some_and(|ext| ext == "jsonl");
            if !item.file_type().is_file() || !is_jsonl {
                continue;
            }
            let meta = match item.metadata() {
                Ok(meta) => meta,
                Err(err) => {
                    complete_scan = false;
                    self.read_errors
                        .insert(item.path().to_path_buf(), err.to_string());
                    continue;
                }
            };
            let path = item.into_path();
            // Only an event naming the file forces a full check. A directory
            // event (a child created or removed) leaves its other files to the
            // stamp; rehashing them would reread the whole project folder.
            let notified = paths.contains(&path);
            let (added, reset) = self.ingest(root, &path, &meta, force, notified);
            changed |= added;
            rebuild |= reset;
            seen.insert(path);
        }

        if complete_scan {
            self.files.retain(|path, state| {
                let keep = !affected(path) || seen.contains(path);
                rebuild |= !keep && !state.entries.is_empty();
                keep
            });
        }
        changed |= rebuild
            || previous_file_count != self.files.len()
            || self.read_errors != previous_errors;
        if rebuild {
            self.rebuild_history();
        }
        if changed {
            self.generation += 1;
            self.project_meta = self.project_meta();
        }
        changed
    }

    pub fn snapshot(&mut self, now: DateTime<Utc>) -> UsageData {
        self.update_readings(now);
        let mut data = self.history.snapshot(&self.project_meta, now, &Local);
        self.update_archive(now);
        self.snapshot_id += 1;
        data.snapshot_id = self.snapshot_id;
        data.data_dirs = self.roots.display();
        data.file_count = self.files.len();
        data.custom_prices = self.pricing.custom_count();
        data.cache_read_price = self.pricing.cache_read_share(
            data.models
                .iter()
                .map(|m| (m.model.as_str(), m.usage.cache_read_tokens)),
        );
        data.pricing_error = self.pricing_error.clone();
        data.read_errors = self
            .roots
            .problem()
            .into_iter()
            .chain(self.read_errors.iter().map(|(path, message)| ReadError {
                path: path.display().to_string(),
                message: message.clone(),
            }))
            .collect();
        data.quota = self.quota_status(
            data.active_block
                .as_ref()
                .map(|b| (b.start_time, b.end_time)),
            now,
        );
        if let Some(archive) = &mut self.archive {
            archive.save();
            data.archive = Some(archive.status(self.history.filled_responses()));
        }
        data
    }

    /// What the archive view shows beyond the snapshot, as of the last refresh.
    pub fn archive_summary(&mut self, now: DateTime<Utc>) -> ArchiveSummary {
        let (restored, blocks) = self.history.archive_view(now, &Local);
        ArchiveSummary {
            generated_at: now,
            daily: self.history.daily_history(now, &Local),
            restored,
            blocks,
            files: self
                .archive
                .as_ref()
                .map(Archive::files)
                .unwrap_or_default(),
            reveal_path: self
                .archive
                .as_ref()
                .map(|archive| archive.reveal_path().display().to_string()),
            retention_days: self.roots.retention_days(),
        }
    }

    /// Brings the archive up to date once a day or a block has ended, and
    /// when responses changed before the latest one did. Call after the
    /// history's snapshot.
    fn update_archive(&mut self, now: DateTime<Utc>) {
        let Some(archive) = &mut self.archive else {
            return;
        };
        let today_start = archive::day_start(now.with_timezone(&Local).date_naive(), &Local);
        let last_ended = self.history.last_ended_block_end(now);
        let frontier = last_ended.map_or(today_start, |end| end.max(today_start));
        let changed = self.history.take_earliest_change();
        if self.archived_through == Some((today_start, last_ended))
            && changed.is_none_or(|at| at >= frontier)
        {
            return;
        }
        self.archived_through = Some((today_start, last_ended));
        let days = archive::merge_days(archive.days(), self.history.entries(), now, &Local);
        let blocks = archive::merge_blocks(archive.blocks(), self.history.ended_blocks(now));
        // The merge reproduces what the history shows, so this snapshot stands.
        if archive.update_usage(days, blocks, &self.project_meta, &self.pricing) {
            self.history.set_archive(archive, &self.pricing);
        }
    }

    /// Reads Desktop's samples, archives new ones, and tells the history in
    /// which spans another account than the current one was signed in.
    /// Call before the history's snapshot.
    fn update_readings(&mut self, now: DateTime<Utc>) {
        let reloaded = self.quota.reload();
        if let Some(archive) = &mut self.archive {
            if (reloaded || self.readings_due)
                && let Some(tag) = self.quota.account()
            {
                archive.add_readings(tag, self.quota.samples(), now);
            }
            self.readings_due = false;
        }
        // The archive remembers readings Desktop's file no longer has.
        let spans = self.account().map_or_else(Vec::new, |account| {
            let desktop = self
                .quota
                .timeline()
                .iter()
                .map(|(at, tag)| (*at, tag.as_str()));
            let archived = self
                .archive
                .iter()
                .flat_map(|archive| archive.reading_times(now));
            quota::foreign_spans(desktop.chain(archived), account)
        });
        self.history.set_foreign(spans);
    }

    /// The account Desktop is signed in to: the one of its latest sample, or
    /// of the archive's latest reading once Desktop's file is gone.
    fn account(&self) -> Option<&str> {
        self.quota
            .account()
            .or_else(|| self.archive.as_ref()?.latest_account())
    }

    fn quota_status(
        &mut self,
        open_block: Option<(DateTime<Utc>, DateTime<Utc>)>,
        now: DateTime<Utc>,
    ) -> Option<quota::QuotaStatus> {
        // With an archive, Desktop's samples join the archived readings, which
        // also outlast them; the estimate reads the last week of those.
        let mut account = None;
        let samples: Cow<'_, [Sample]> = match &self.archive {
            None => Cow::Borrowed(self.quota.samples()),
            Some(archive) => {
                let tag = self.account()?.to_owned();
                let samples = archive.recent_samples(&tag, now);
                account = Some(tag);
                Cow::Owned(samples)
            }
        };
        let first = samples.first()?.at;
        // Calibration needs the sampled period; the pace needs the open block.
        let since = open_block.map_or(first, |(start, _)| start.min(first));
        let fit = QuotaFit::update(
            self.quota_fit.take(),
            self.generation,
            since,
            samples,
            &self.history,
        );
        let fit = self.quota_fit.insert(fit);
        let block = open_block.map(|(start, end)| LocalBlock { start, end });
        let last_end = self.history.last_block().map(|(_, end)| end);
        let archived = match (&self.archive, &account) {
            (Some(archive), Some(tag)) => archive.calibration(tag, now),
            _ => PercentPerUsd::default(),
        };
        let mut status = quota::estimate(
            &fit.samples,
            &fit.spend,
            fit.fitted,
            block,
            last_end,
            archived,
            now,
        )?;
        if let (Some(archive), Some(tag)) = (&mut self.archive, &account) {
            archive.record_calibration(tag, status.fitted, status.sampled_at);
        }
        if let Some(week) = status.week.as_mut() {
            self.attribute_week(week);
        }
        Some(status)
    }

    /// This machine's spend under the current account since the weekly
    /// period began, in percentage points of the weekly limit, per project
    /// and per model.
    fn attribute_week(&self, week: &mut quota::WeekStatus) {
        let Some(k) = week.percent_per_usd else {
            return;
        };
        let (projects, models) = self.history.quota_spend_by_source(week.period_start);
        week.projects = quota::week_shares(
            projects.into_iter().map(|(id, usd)| {
                let name = self
                    .project_meta
                    .get(&id)
                    .map_or_else(|| id.to_string(), |meta| meta.name.clone());
                (id.to_string(), name, usd)
            }),
            k,
        );
        week.models = quota::week_shares(
            models
                .into_iter()
                .map(|(model, usd)| (model.clone(), model, usd)),
            k,
        );
    }

    fn ingest(
        &mut self,
        root: &Path,
        path: &Path,
        meta: &Metadata,
        force: bool,
        notified: bool,
    ) -> (bool, bool) {
        let state = self
            .files
            .entry(path.to_path_buf())
            .or_insert_with(|| FileState::new(root, path));
        let stamp = FileStamp::from(meta);
        if !force && !notified && !state.failed && state.stamp.as_ref() == Some(&stamp) {
            return (false, false);
        }
        let mut changed = false;
        let mut reset = false;
        let history = &mut self.history;
        let pricing = &self.pricing;
        let result = (|| -> io::Result<()> {
            // Open before clearing anything, preserving cached usage if a
            // temporary sharing/permission error prevents reading the file.
            let mut file = File::open(path)?;
            let stamp = FileStamp::from(&file.metadata()?);
            let replaced = state
                .stamp
                .as_ref()
                .is_some_and(|old| !old.same_file(&stamp));
            let reload = force
                || replaced
                || stamp.len < state.offset
                || if state.failed || stamp.len <= state.offset {
                    !state.prefix_matches(&mut file)?
                } else {
                    // Growth is normally an append. Samples detect common rewrites
                    // cheaply; the background validator covers the entire prefix.
                    state.anchors.is_some_and(|_| state.offset > 0)
                        && Some(prefix_anchors(&mut file, state.offset)?) != state.anchors
                };
            if reload {
                // Commit a replacement only after a successful read. A failed
                // rescan must leave the last successfully parsed data intact.
                let mut replacement = FileState::new(root, path);
                let mut replacement_changed = false;
                replacement.stamp = Some(stamp);
                replacement.read_new_lines(
                    &mut file,
                    pricing,
                    &mut |_| {},
                    &mut replacement_changed,
                )?;
                replacement.verified = replacement.offset;
                replacement.anchors = Some(prefix_anchors(&mut file, replacement.offset)?);
                reset = !state.entries.is_empty() || !replacement.entries.is_empty();
                changed |= state.cwd != replacement.cwd;
                *state = replacement;
            } else {
                state.stamp = Some(stamp);
                let first_read = state.offset == 0;
                // Apply complete append records as they arrive, even if a
                // later read fails; don't retain superseded streaming chunks.
                state.read_new_lines(
                    &mut file,
                    pricing,
                    &mut |entry| {
                        history.insert(entry, &Local);
                    },
                    &mut changed,
                )?;
                if first_read {
                    state.verified = state.offset;
                }
                state.anchors = Some(prefix_anchors(&mut file, state.offset)?);
            }
            Ok(())
        })();
        state.failed = result.is_err();
        if let Err(err) = result {
            eprintln!("cc-deskmeter: failed to read {}: {err}", path.display());
            self.read_errors.insert(path.to_path_buf(), err.to_string());
        }
        (changed, reset)
    }

    /// Display name and path per project folder. Prefers a `cwd` that maps back
    /// to the folder name exactly, then the shortest (closest to the root);
    /// the archive names projects whose transcripts are gone.
    fn project_meta(&self) -> HashMap<Arc<str>, ProjectMeta> {
        let mut meta = self.live_project_meta();
        if let Some(archive) = &self.archive {
            for (id, info) in archive.projects() {
                if !meta.contains_key(id.as_str()) {
                    meta.insert(Arc::from(id.as_str()), info.clone());
                }
            }
        }
        meta
    }

    fn live_project_meta(&self) -> HashMap<Arc<str>, ProjectMeta> {
        let mut best: HashMap<Arc<str>, (&str, bool)> = HashMap::new();
        for state in self.files.values() {
            let Some(cwd) = state.cwd.as_deref() else {
                continue;
            };
            let candidate = (cwd, state.cwd_exact);
            match best.entry(Arc::clone(&state.project)) {
                Entry::Vacant(slot) => {
                    slot.insert(candidate);
                }
                Entry::Occupied(mut slot) => {
                    let (current, current_exact) = *slot.get();
                    if (candidate.1, std::cmp::Reverse(cwd.len()))
                        > (current_exact, std::cmp::Reverse(current.len()))
                        || (candidate.1 == current_exact
                            && cwd.len() == current.len()
                            && cwd < current)
                    {
                        slot.insert(candidate);
                    }
                }
            }
        }
        best.into_iter()
            .map(|(project, (cwd, _))| {
                let name = cwd
                    .rsplit(['/', '\\'])
                    .find(|segment| !segment.is_empty())
                    .unwrap_or(cwd);
                let meta = ProjectMeta {
                    name: name.to_owned(),
                    path: Some(cwd.to_owned()),
                };
                (project, meta)
            })
            .collect()
    }
}

/// What the quota estimate keeps between snapshots: the quota spend since
/// the readings began (collecting and sorting a week of responses is the
/// costly part) and `k` fitted on it. The spend holds while usage (the
/// store's generation), its window (`since`) and the other accounts' spans
/// stay; the fit while the readings stay too. Only the rest of the estimate
/// depends on the time.
struct QuotaFit {
    generation: u64,
    since: DateTime<Utc>,
    foreign: Vec<quota::Span>,
    spend: Arc<Spend>,
    samples: Vec<Sample>,
    fitted: PercentPerUsd,
}

impl QuotaFit {
    /// `cached` if it still holds for these inputs; otherwise fitted again,
    /// on its spend if only the readings changed.
    fn update(
        cached: Option<Self>,
        generation: u64,
        since: DateTime<Utc>,
        samples: Cow<'_, [Sample]>,
        history: &History,
    ) -> Self {
        let spend = match cached {
            Some(fit)
                if fit.generation == generation
                    && fit.since == since
                    && fit.foreign == history.foreign() =>
            {
                if fit.samples[..] == samples[..] {
                    return fit;
                }
                fit.spend
            }
            _ => Arc::new(Spend::new(history.quota_spend_since(since))),
        };
        Self {
            generation,
            since,
            foreign: history.foreign().to_vec(),
            fitted: quota::fit(&samples, &spend),
            spend,
            samples: samples.into_owned(),
        }
    }
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl FileStamp {
    fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        if self.identity != other.identity {
            return false;
        }
        self.created == other.created
    }
}

/// A full-prefix audit can run without holding the UsageStore mutex.
pub struct ValidationJob {
    path: PathBuf,
    offset: u64,
    checksum: u32,
    incarnation: Arc<()>,
}

pub struct ValidationResult {
    job: ValidationJob,
    result: Result<bool, String>,
}

impl ValidationJob {
    pub fn check(self) -> ValidationResult {
        let result = File::open(&self.path)
            .and_then(|mut file| prefix_checksum(&mut file, self.offset))
            .map(|checksum| checksum == Some(self.checksum))
            .map_err(|err| err.to_string());
        ValidationResult { job: self, result }
    }
}

fn prefix_checksum(file: &mut File, offset: u64) -> io::Result<Option<u32>> {
    file.seek(SeekFrom::Start(0))?;
    let mut prefix = file.take(offset);
    let mut checksum = crc32fast::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = prefix.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        checksum.update(&buffer[..read]);
    }
    Ok((prefix.limit() == 0).then(|| checksum.finalize()))
}

fn prefix_anchors(file: &mut File, offset: u64) -> io::Result<(u32, u32)> {
    let mut buffer = [0u8; 4096];
    let len = offset.min(buffer.len() as u64) as usize;
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut buffer[..len])?;
    let head = crc32fast::hash(&buffer[..len]);
    file.seek(SeekFrom::Start(offset - len as u64))?;
    file.read_exact(&mut buffer[..len])?;
    Ok((head, crc32fast::hash(&buffer[..len])))
}

impl From<&Metadata> for FileStamp {
    fn from(meta: &Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino())
            },
        }
    }
}

struct FileState {
    incarnation: Arc<()>,
    project: Arc<str>,
    source: Arc<str>,
    /// Session id used when a line has no `sessionId` (the file stem).
    fallback_session: String,
    /// Bytes consumed so far; always on a line boundary.
    offset: u64,
    /// Prefix known to match the file: read in one pass or audited since.
    verified: u64,
    entries: HashMap<Arc<str>, Arc<UsageEntry>>,
    stamp: Option<FileStamp>,
    checksum: crc32fast::Hasher,
    anchors: Option<(u32, u32)>,
    failed: bool,
    cwd: Option<String>,
    /// Whether `cwd` encodes to the project folder name.
    cwd_exact: bool,
    records: HashMap<Arc<str>, RecordLink>,
    /// `records` keys, oldest first; bounded by `MAX_RECORD_LINKS`.
    record_order: VecDeque<Arc<str>>,
    /// Fallback for older transcripts without UUID links, scoped by session.
    last_inputs: HashMap<String, DateTime<Utc>>,
}

struct RecordLink {
    parent_uuid: Option<String>,
    session_id: Option<String>,
    input_time: Option<DateTime<Utc>>,
}

impl FileState {
    fn new(root: &Path, path: &Path) -> Self {
        Self {
            incarnation: Arc::new(()),
            project: project_of(root, path),
            source: Arc::from(path.to_string_lossy().as_ref()),
            fallback_session: path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default(),
            offset: 0,
            verified: 0,
            entries: HashMap::new(),
            stamp: None,
            checksum: crc32fast::Hasher::new(),
            anchors: None,
            failed: false,
            cwd: None,
            cwd_exact: false,
            records: HashMap::new(),
            record_order: VecDeque::new(),
            last_inputs: HashMap::new(),
        }
    }

    fn prefix_matches(&self, file: &mut File) -> io::Result<bool> {
        Ok(prefix_checksum(file, self.offset)? == Some(self.checksum.clone().finalize()))
    }

    /// Parses complete lines after `offset`. A trailing line without `\n` is
    /// taken only if it is already valid JSON; otherwise it is still being
    /// written and is re-read on the next refresh.
    fn read_new_lines(
        &mut self,
        file: &mut File,
        pricing: &Pricing,
        on_entry: &mut impl FnMut(Arc<UsageEntry>),
        changed: &mut bool,
    ) -> io::Result<()> {
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();

        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            let complete =
                line.last() == Some(&b'\n') || serde_json::from_slice::<IgnoredAny>(&line).is_ok();
            if !complete {
                break;
            }
            *changed |= self.ingest_line(&line, pricing, on_entry);
            self.checksum.update(&line);
            self.offset += read as u64;
        }
        Ok(())
    }

    /// `self.offset` must still point at the start of `line`.
    fn ingest_line(
        &mut self,
        line: &[u8],
        pricing: &Pricing,
        on_entry: &mut impl FnMut(Arc<UsageEntry>),
    ) -> bool {
        let Ok(line) = std::str::from_utf8(line) else {
            return false;
        };
        let Some(parsed) = parser::parse_line(line) else {
            if let Some(context) = parser::parse_context(line) {
                self.note_record(context);
            }
            return false;
        };
        self.note_record(RecordContext {
            kind: Some("assistant".into()),
            uuid: parsed.uuid.clone(),
            parent_uuid: parsed.parent_uuid.clone(),
            session_id: parsed.session_id.clone(),
            timestamp: Some(parsed.timestamp),
        });
        let meta_changed = parsed.cwd.as_deref().is_some_and(|cwd| self.note_cwd(cwd));
        let entry = self.to_entry(parsed, pricing);
        if self
            .entries
            .get(&entry.dedup_key)
            .is_some_and(|old| !entry.precedence(old).is_gt())
        {
            return meta_changed;
        }
        let entry = Arc::new(entry);
        self.entries
            .insert(entry.dedup_key.clone(), Arc::clone(&entry));
        on_entry(entry);
        true
    }

    fn note_record(&mut self, context: RecordContext) {
        let input_time = context.input_time();
        if let Some(time) = input_time {
            self.last_inputs
                .insert(context.session_id.clone().unwrap_or_default(), time);
        }
        if let Some(uuid) = context.uuid {
            let uuid: Arc<str> = Arc::from(uuid);
            let link = RecordLink {
                parent_uuid: context.parent_uuid,
                session_id: context.session_id,
                input_time,
            };
            if self.records.insert(Arc::clone(&uuid), link).is_none() {
                self.record_order.push_back(uuid);
                if self.record_order.len() > MAX_RECORD_LINKS
                    && let Some(oldest) = self.record_order.pop_front()
                {
                    self.records.remove(&oldest);
                }
            }
        }
    }

    fn request_time(&self, parsed: &ParsedUsage) -> Option<DateTime<Utc>> {
        if let Some(mut parent) = parsed.parent_uuid.as_deref() {
            // The bound also protects against malformed cyclic parent links.
            for _ in 0..self.records.len() {
                let record = self.records.get(parent)?;
                if record
                    .session_id
                    .as_ref()
                    .zip(parsed.session_id.as_ref())
                    .is_some_and(|(a, b)| a != b)
                {
                    return None;
                }
                if let Some(time) = record.input_time {
                    return (time <= parsed.timestamp).then_some(time);
                }
                parent = record.parent_uuid.as_deref()?;
            }
            return None;
        }
        self.last_inputs
            .get(parsed.session_id.as_deref().unwrap_or_default())
            .or_else(|| self.last_inputs.get(""))
            .copied()
            .filter(|time| *time <= parsed.timestamp)
    }

    fn note_cwd(&mut self, cwd: &str) -> bool {
        if self.cwd_exact {
            return false;
        }
        let exact = encodes_to(cwd, &self.project);
        if exact || self.cwd.is_none() {
            self.cwd = Some(cwd.to_owned());
            self.cwd_exact = exact;
            return true;
        }
        false
    }

    fn to_entry(&self, parsed: ParsedUsage, pricing: &Pricing) -> UsageEntry {
        let request_time = self.request_time(&parsed);
        let price = price_of(
            pricing,
            &parsed.model,
            &parsed.tokens,
            parsed.cache_write_1h,
            parsed.fast,
        );
        // CLAUDE.md: deduplicate by `message.id`. Lines without one fall back to
        // `requestId`, then to their byte position so they are never merged.
        let dedup_key: Arc<str> = parsed
            .message_id
            .or_else(|| parsed.request_id.map(|id| format!("req:{id}")))
            .unwrap_or_else(|| format!("{}#{}", self.source, self.offset))
            .into();
        // Later chunks of a streamed response keep the request time of the
        // first, even when tool results were logged in between.
        let requested_at = match self.entries.get(&dedup_key) {
            Some(first) => first.requested_at,
            None => request_time.unwrap_or(parsed.timestamp),
        }
        .min(parsed.timestamp);

        price.apply(UsageEntry {
            dedup_key,
            source: Arc::clone(&self.source),
            offset: self.offset,
            complete: parsed.complete,
            timestamp: parsed.timestamp,
            requested_at,
            model: parsed.model,
            project: Arc::clone(&self.project),
            session_id: parsed
                .session_id
                .unwrap_or_else(|| self.fallback_session.clone()),
            tokens: parsed.tokens,
            cache_write_1h: parsed.cache_write_1h,
            fast: parsed.fast,
            long_context: false,
            cost_usd: 0.0,
            quota_usd: 0.0,
            priced: false,
        })
    }
}

/// The priced fields of a response, at some prices.
struct Price {
    long_context: bool,
    cost_usd: f64,
    quota_usd: f64,
    priced: bool,
}

impl Price {
    fn matches(&self, entry: &UsageEntry) -> bool {
        (
            self.long_context,
            self.cost_usd,
            self.quota_usd,
            self.priced,
        ) == (
            entry.long_context,
            entry.cost_usd,
            entry.quota_usd,
            entry.priced,
        )
    }

    fn apply(self, entry: UsageEntry) -> UsageEntry {
        UsageEntry {
            long_context: self.long_context,
            cost_usd: self.cost_usd,
            quota_usd: self.quota_usd,
            priced: self.priced,
            ..entry
        }
    }
}

/// What a response of `model` with these tokens costs at `pricing`: when
/// first read and again whenever prices change.
fn price_of(
    pricing: &Pricing,
    model: &str,
    tokens: &TokenCounts,
    cache_write_1h: u64,
    fast: bool,
) -> Price {
    let long_context = pricing
        .long_context_threshold(model)
        .is_some_and(|threshold| tokens.prompt() > threshold);
    let rates = pricing.rates_for(model);
    let (cost_usd, quota_usd) = rates.map_or((0.0, 0.0), |rates| {
        (
            pricing::tier_cost_usd(&rates, tokens, cache_write_1h, fast, long_context),
            quota::quota_weight(&rates, tokens, cache_write_1h, fast, long_context),
        )
    });
    Price {
        long_context,
        cost_usd,
        quota_usd,
        priced: rates.is_some(),
    }
}

/// First path component under the root, i.e. Claude Code's project folder.
fn project_of(root: &Path, path: &Path) -> Arc<str> {
    let mut components = path.strip_prefix(root).unwrap_or(path).components();
    match (components.next(), components.next()) {
        (Some(folder), Some(_)) => Arc::from(folder.as_os_str().to_string_lossy().as_ref()),
        _ => Arc::from("unknown"),
    }
}

/// Claude Code names a project folder after its cwd with every
/// non-alphanumeric character replaced by `-`.
fn encodes_to(cwd: &str, folder: &str) -> bool {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .eq(folder.chars())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use chrono::TimeDelta;

    use super::*;

    fn line(id: &str, ts: &str, cwd: &str, input: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"s1","cwd":"{cwd}","requestId":"req_{id}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":{input},"output_tokens":0}}}}}}"#
        )
    }

    fn append(path: &Path, text: &str) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
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

    #[test]
    fn reads_incrementally_and_waits_for_partial_lines() {
        let tmp = TempDir::new("incremental");
        let project = tmp.0.join("-work-my-app");
        fs::create_dir_all(project.join("sub")).unwrap();
        let file = project.join("s1.jsonl");
        let cwd = "/work/my-app";

        append(
            &file,
            &format!(
                "{}\n{}\n",
                line("m1", "2026-10-17T10:00:00Z", "/work/my-app/src", 10),
                line("m1", "2026-10-17T10:00:01Z", cwd, 10)
            ),
        );
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        assert!(store.refresh());
        assert!(!store.refresh(), "nothing new to read");

        // A half-written line is left for later.
        let next = line("m2", "2026-10-17T10:01:00Z", cwd, 5);
        let (head, rest) = next.split_at(40);
        append(&file, head);
        assert!(!store.refresh());
        append(&file, &format!("{rest}\n"));
        assert!(store.refresh());

        // Subagent transcripts in nested folders belong to the same project.
        append(
            &project.join("sub").join("agent.jsonl"),
            &format!("{}\n", line("m3", "2026-10-17T10:02:00Z", cwd, 1)),
        );
        assert!(store.refresh());

        let data = store.snapshot(
            DateTime::parse_from_rfc3339("2026-10-17T10:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        assert_eq!(data.file_count, 2);
        assert_eq!(data.total.message_count, 3, "m1 is deduplicated");
        assert_eq!(data.total.input_tokens, 16);
        assert_eq!(data.projects.len(), 1);
        assert_eq!(data.projects[0].id, "-work-my-app");
        assert_eq!(data.projects[0].name, "my-app");
        assert_eq!(data.projects[0].path.as_deref(), Some(cwd));

        // Truncation resets the file; deletion drops its entries.
        fs::write(&file, "").unwrap();
        assert!(store.refresh());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 1);
        fs::remove_file(project.join("sub").join("agent.jsonl")).unwrap();
        assert!(store.refresh());
        assert_eq!(store.snapshot(Utc::now()).total.message_count, 0);
    }

    fn set_modified(path: &Path, time: SystemTime) {
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(time))
            .unwrap();
    }

    #[test]
    fn reloads_equal_length_larger_and_middle_rewrites() {
        let tmp = TempDir::new("rewrites");
        let project = tmp.0.join("project");
        fs::create_dir_all(&project).unwrap();
        let file = project.join("s.jsonl");
        let record = |tokens| {
            format!(
                "{}\n",
                line("same", "2026-10-17T10:00:00Z", "/project", tokens)
            )
        };
        fs::write(&file, record(10)).unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10);
        for (i, tokens) in [90, 900].into_iter().enumerate() {
            fs::write(&file, record(tokens)).unwrap();
            set_modified(
                &file,
                SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100 + i as u64),
            );
            assert!(store.refresh());
            assert_eq!(store.snapshot(Utc::now()).total.input_tokens, tokens);
        }

        // The changed record is outside both a 64-KiB head and tail sample.
        let padding = format!(
            "{{\"type\":\"user\",\"content\":\"{}\"}}\n",
            "x".repeat(70_000)
        );
        fs::write(&file, format!("{padding}{}{padding}", record(10))).unwrap();
        store.refresh();
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10);
        fs::write(
            &file,
            format!(
                "{padding}{}{padding}{}\n",
                record(90),
                line("new", "2026-10-17T10:01:00Z", "/project", 1)
            ),
        )
        .unwrap();
        assert!(store.refresh());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 11);
        // Appends stay bounded; the routine audit of the grown transcript
        // catches the middle rewrite.
        let jobs = store.validation_jobs(false);
        assert!(
            jobs.into_iter()
                .any(|job| store.apply_validation(job.check()))
        );
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 91);
        assert!(!store.refresh());
    }

    #[test]
    fn notifications_and_manual_rescan_handle_preserved_metadata() {
        let tmp = TempDir::new("preserved-metadata");
        let file = tmp.0.join("s.jsonl");
        let record = |tokens| {
            format!(
                "{}\n",
                line("same", "2026-10-17T10:00:00Z", "/project", tokens)
            )
        };
        fs::write(&file, record(10)).unwrap();
        let modified = file.metadata().unwrap().modified().unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        fs::write(&file, record(90)).unwrap();
        set_modified(&file, modified);
        assert!(store.refresh_paths(&HashSet::from([file.clone()])));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 90);
        fs::write(&file, record(50)).unwrap();
        set_modified(&file, modified);
        assert!(store.rescan());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 50);
    }

    #[test]
    fn stable_cross_file_winner_and_deleted_winner_falls_back() {
        let tmp = TempDir::new("stable-dedup");
        let partial = tmp.0.join("z-partial.jsonl");
        let complete = tmp.0.join("a-complete.jsonl");
        fs::write(
            &partial,
            format!("{}\n", line("same", "2026-10-17T10:00:00Z", "/project", 1)),
        )
        .unwrap();
        let final_record = line("same", "2026-10-17T10:00:00Z", "/project", 100)
            .replace("\"usage\":", "\"stop_reason\":\"end_turn\",\"usage\":");
        fs::write(&complete, format!("{final_record}\n")).unwrap();
        for _ in 0..32 {
            let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
            store.refresh();
            assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 100);
        }
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        fs::remove_file(&complete).unwrap();
        assert!(store.refresh());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 1);
        append(
            &partial,
            &format!("{}\n", line("same", "2026-10-17T10:00:01Z", "/project", 5)),
        );
        store.refresh();
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.input_tokens, 5);
        assert_eq!(data.total.message_count, 1);
        assert_eq!(
            store.files[&partial].entries.len(),
            1,
            "discard superseded streaming chunks"
        );
    }

    #[test]
    fn responses_take_the_request_time_from_the_preceding_record() {
        let tmp = TempDir::new("request-time");
        let file = tmp.0.join("s.jsonl");
        let record = |ts: &str| format!(r#"{{"type":"user","uuid":"u","timestamp":"{ts}"}}"#);
        let lines = [
            record("2026-10-18T01:51:59.025Z"),
            line("m1", "2026-10-18T01:52:01.772Z", "/p", 1),
            // A tool result logged between two chunks of the same response.
            record("2026-10-18T01:52:02.500Z"),
            line("m1", "2026-10-18T01:52:03Z", "/p", 2),
            line("m2", "2026-10-18T01:52:07Z", "/p", 3),
            // A long response still belongs to its original request.
            record("2026-10-18T02:00:00Z"),
            line("m3", "2026-10-18T02:30:00Z", "/p", 4),
        ];
        fs::write(&file, lines.join("\n") + "\n").unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        let requested = |id: &str| store.files[&file].entries[id].requested_at;
        assert_eq!(
            requested("m1"),
            DateTime::parse_from_rfc3339("2026-10-18T01:51:59.025Z").unwrap()
        );
        assert_eq!(
            requested("m2"),
            DateTime::parse_from_rfc3339("2026-10-18T01:52:02.500Z").unwrap()
        );
        assert_eq!(
            requested("m3"),
            DateTime::parse_from_rfc3339("2026-10-18T02:00:00Z").unwrap()
        );
        let block = store
            .snapshot(
                DateTime::parse_from_rfc3339("2026-10-18T04:10:20Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )
            .active_block
            .expect("block still open");
        assert_eq!(block.start_time.to_rfc3339(), "2026-10-18T01:50:00+00:00");
        assert_eq!(block.end_time.to_rfc3339(), "2026-10-18T06:50:00+00:00");
    }

    #[test]
    fn linked_requests_ignore_progress_and_interleaved_sessions_at_block_boundary() {
        let tmp = TempDir::new("linked-requests");
        let file = tmp.0.join("s.jsonl");
        let response = |id, ts, parent: &str| {
            let mut record: serde_json::Value =
                serde_json::from_str(&line(id, ts, "/p", 1)).unwrap();
            record["uuid"] = format!("uuid-{id}").into();
            record["parentUuid"] = parent.into();
            record.to_string()
        };
        let lines = [
            r#"{"type":"user","uuid":"first","sessionId":"s1","timestamp":"2026-10-18T10:00:00Z"}"#.into(),
            response("first", "2026-10-18T10:01:00Z", "first"),
            r#"{"type":"user","uuid":"prompt","sessionId":"s1","timestamp":"2026-10-18T14:59:00Z"}"#.into(),
            r#"{"type":"user","uuid":"unrelated","sessionId":"s2","timestamp":"2026-10-18T15:00:05Z"}"#.into(),
            r#"{"type":"progress","uuid":"progress","parentUuid":"prompt","sessionId":"s1","data":{"timestamp":"2099-01-01T00:00:00Z"},"timestamp" : "2026-10-18T15:00:30Z"}"#.into(),
            r#"{"type":"system","timestamp":"2026-10-18T15:00:40Z"}"#.into(),
            response("linked", "2026-10-18T15:01:00Z", "progress"),
            // Old transcripts without parent UUIDs still get session-scoped input times.
            line("legacy", "2026-10-18T15:02:00Z", "/p", 1),
        ];
        fs::write(&file, lines.join("\n") + "\n").unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        let expected = DateTime::parse_from_rfc3339("2026-10-18T14:59:00Z").unwrap();
        for id in ["linked", "legacy"] {
            assert_eq!(store.files[&file].entries[id].requested_at, expected);
        }
        let data = store.snapshot(
            DateTime::parse_from_rfc3339("2026-10-18T15:03:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        assert!(
            data.active_block.is_none(),
            "background records must not open a new block"
        );
        assert_eq!(data.peak_block.unwrap().usage.message_count, 3);
    }

    #[test]
    fn explicit_missing_or_cyclic_parents_do_not_borrow_an_unrelated_prompt() {
        let tmp = TempDir::new("broken-parent-links");
        let file = tmp.0.join("s.jsonl");
        let mut state = FileState::new(&tmp.0, &file);
        for json in [
            r#"{"type":"user","uuid":"prompt","sessionId":"s1","timestamp":"2026-10-18T10:00:00Z"}"#,
            r#"{"type":"progress","uuid":"cycle","parentUuid":"cycle"}"#,
        ] {
            state.note_record(parser::parse_context(json).unwrap());
        }
        for parent in ["missing", "cycle"] {
            let mut parsed =
                parser::parse_line(&line("m", "2026-10-18T11:00:00Z", "/p", 1)).unwrap();
            parsed.parent_uuid = Some(parent.into());
            let timestamp = parsed.timestamp;
            assert_eq!(
                state.to_entry(parsed, &Pricing::default()).requested_at,
                timestamp
            );
        }
    }

    #[test]
    fn snapshot_order_does_not_depend_on_clock_or_usage_changes() {
        let mut store = UsageStore::new(Roots::default());
        let now = Utc::now();
        let first = store.snapshot(now);
        let second = store.snapshot(now);
        let third = store.snapshot(now - chrono::TimeDelta::hours(1));
        assert!(first.snapshot_id < second.snapshot_id && second.snapshot_id < third.snapshot_id);
        assert_eq!(store.generation(), 0);
    }

    #[test]
    fn targeted_refresh_keeps_other_files_and_handles_directory_deletion() {
        let tmp = TempDir::new("targeted-refresh");
        let folder = tmp.0.join("nested");
        fs::create_dir(&folder).unwrap();
        let a = folder.join("a.jsonl");
        let b = tmp.0.join("b.jsonl");
        for path in [&a, &b] {
            fs::write(
                path,
                format!(
                    "{}\n",
                    line(
                        path.file_stem().unwrap().to_str().unwrap(),
                        "2026-10-18T10:00:00Z",
                        "/p",
                        10
                    )
                ),
            )
            .unwrap();
        }
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        append(
            &a,
            &format!("{}\n", line("a-new", "2026-10-18T10:01:00Z", "/p", 1)),
        );
        append(
            &b,
            &format!("{}\n", line("b-new", "2026-10-18T10:01:00Z", "/p", 2)),
        );
        store.refresh_paths(&HashSet::from([a.clone()]));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 21);
        fs::remove_file(&a).unwrap();
        fs::remove_dir(&folder).unwrap();
        store.refresh_paths(&HashSet::from([folder]));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10);
        store.refresh();
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 12);
    }

    #[test]
    fn reads_every_root_once_and_picks_up_roots_created_later() {
        let tmp = TempDir::new("multi-root");
        let (legacy, xdg) = (tmp.0.join("legacy"), tmp.0.join("xdg"));
        let a = legacy.join("-work-a").join("s.jsonl");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::write(
            &a,
            format!(
                "{}\n{}\n",
                line("shared", "2026-10-18T10:00:00Z", "/work/a", 10),
                line("only-a", "2026-10-18T10:00:30Z", "/work/a", 1)
            ),
        )
        .unwrap();
        let mut roots = Roots::from(xdg.clone());
        roots.dirs.push(legacy.clone());
        let mut store = UsageStore::new(roots);
        assert!(store.refresh(), "a missing root does not hide the other");
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 11);

        // The second root appears later, with a copy of the same response.
        let b = xdg.join("-work-b").join("s.jsonl");
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::write(
            &b,
            format!(
                "{}\n{}\n",
                line("shared", "2026-10-18T10:00:00Z", "/work/a", 10),
                line("only-b", "2026-10-18T10:01:00Z", "/work/b", 5)
            ),
        )
        .unwrap();
        assert!(store.refresh_paths(&HashSet::from([b.clone()])));
        let data = store.snapshot(Utc::now());
        assert_eq!(data.file_count, 2);
        assert_eq!(data.total.input_tokens, 16, "deduplicated across roots");
        let mut projects: Vec<_> = data.projects.iter().map(|p| p.id.as_str()).collect();
        projects.sort();
        assert_eq!(
            projects,
            ["-work-a", "-work-b"],
            "project folders are per root"
        );
        assert!(
            data.data_dirs
                .iter()
                .all(|dir| dir.ends_with("xdg") || dir.ends_with("legacy"))
        );

        // Paths outside every root are ignored.
        assert!(!store.refresh_paths(&HashSet::from([tmp.0.join("elsewhere.jsonl")])));
        fs::remove_dir_all(&xdg).unwrap();
        assert!(store.refresh());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 11);
    }

    #[test]
    fn local_prices_apply_on_refresh_and_bad_files_keep_the_last_valid_ones() {
        let tmp = TempDir::new("pricing-file");
        let logs = tmp.0.join("projects");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("s.jsonl"),
            format!("{}\n", line("m", "2026-10-18T10:00:00Z", "/p", 1_000_000)),
        )
        .unwrap();
        let prices = tmp.0.join("pricing.json");
        let mut store = UsageStore::new(Roots::from(logs)).with_pricing_file(Some(prices.clone()));
        let cost = |store: &mut UsageStore| store.snapshot(Utc::now()).total.cost_usd;

        store.refresh();
        assert!(
            (cost(&mut store) - 4.0).abs() < 1e-9,
            "no file: built-in price"
        );
        assert_eq!(store.snapshot(Utc::now()).custom_prices, 0);

        fs::write(
            &prices,
            r#"{ "claude-opus-5-5": { "input": 9, "output": 45 } }"#,
        )
        .unwrap();
        assert!(
            store.refresh(),
            "an edited price file reprices without new usage"
        );
        let data = store.snapshot(Utc::now());
        assert!((data.total.cost_usd - 9.0).abs() < 1e-9);
        assert_eq!((data.custom_prices, data.pricing_error), (1, None));
        assert!(!store.refresh(), "unchanged file is not reparsed");

        fs::write(&prices, r#"{ "claude-opus-5-5": { "input": 9 "#).unwrap();
        let generation = store.generation();
        assert!(!store.refresh(), "prices are unchanged");
        assert!(
            store.generation() > generation,
            "the error is still published"
        );
        let data = store.snapshot(Utc::now());
        assert!(
            (data.total.cost_usd - 9.0).abs() < 1e-9,
            "last valid prices stay"
        );
        assert!(data.pricing_error.is_some());

        fs::remove_file(&prices).unwrap();
        assert!(store.refresh());
        let data = store.snapshot(Utc::now());
        assert!((data.total.cost_usd - 4.0).abs() < 1e-9);
        assert_eq!((data.custom_prices, data.pricing_error), (0, None));
    }

    #[test]
    fn weekly_use_is_attributed_to_projects_and_models() {
        let tmp = TempDir::new("weekly-attribution");
        let logs = tmp.0.join("projects");
        // $4 per million input tokens (Opus 5.5).
        for (folder, cwd, records) in [
            (
                "-work-alpha",
                "/work/alpha",
                vec![
                    ("a1", "10:20", 1_000_000),
                    ("a2", "10:50", 1_000_000),
                    ("a3", "11:30", 500_000),
                ],
            ),
            ("-work-beta", "/work/beta", vec![("b1", "10:35", 1_000_000)]),
        ] {
            let dir = logs.join(folder);
            fs::create_dir_all(&dir).unwrap();
            let lines: Vec<String> = records
                .iter()
                .map(|(id, time, input)| line(id, &format!("2026-10-18T{time}:00Z"), cwd, *input))
                .collect();
            fs::write(dir.join("s.jsonl"), lines.join("\n") + "\n").unwrap();
        }
        // Desktop's readings: a reset at 10:15, then 2 points per $4 response.
        let at = |time: &str| {
            DateTime::parse_from_rfc3339(&format!("2026-10-18T{time}:00Z"))
                .unwrap()
                .with_timezone(&Utc)
        };
        let samples: Vec<String> = [
            ("10:00", 10),
            ("10:15", 1),
            ("10:30", 3),
            ("10:45", 5),
            ("11:00", 7),
        ]
        .iter()
        .map(|(time, pct)| {
            let t = at(time).timestamp_millis();
            format!(r#"{{"t":{t},"org":"a","u":{{"fh":{pct},"sd":{pct}}}}}"#)
        })
        .collect();
        let history = tmp.0.join("plan-usage-history.json");
        fs::write(
            &history,
            format!(r#"{{"version":2,"samples":[{}]}}"#, samples.join(",")),
        )
        .unwrap();

        let mut store = UsageStore::new(Roots::from(logs)).with_quota_files(vec![history]);
        store.refresh();
        let week = store.snapshot(at("12:00")).quota.unwrap().week.unwrap();
        assert_eq!(week.period_start, at("10:00"));
        let rows = |shares: &[quota::WeekShare]| -> Vec<(String, f64)> {
            shares
                .iter()
                .map(|s| (s.name.clone(), (s.percent * 1e6).round() / 1e6))
                .collect()
        };
        assert_eq!(
            rows(&week.projects),
            [("alpha".to_owned(), 5.0), ("beta".to_owned(), 2.0)]
        );
        assert_eq!(rows(&week.models), [("claude-opus-5-5".to_owned(), 7.0)]);
        // 7 % at 11:00 plus the $2 since, from ~0 % at 10:00.
        assert!((week.pace_per_hour.unwrap() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_switch_of_account_starts_its_own_block_and_spend() {
        let tmp = TempDir::new("account-switch");
        let logs = tmp.0.join("projects").join("-work-alpha");
        fs::create_dir_all(&logs).unwrap();
        let transcript = logs.join("s.jsonl");
        let at = |time: &str| at_utc(&format!("2026-10-18T{time}:00Z"));
        // $4 a response: account a's, then b's after Desktop signed in to b.
        let lines = |times: &[&str]| -> String {
            times
                .iter()
                .map(|time| {
                    let ts = format!("2026-10-18T{time}:00Z");
                    line(&format!("m{time}"), &ts, "/work/alpha", 1_000_000) + "\n"
                })
                .collect()
        };
        fs::write(&transcript, lines(&["08:05", "09:05"])).unwrap();
        let sample = |org: &str, time: &str, pct: u32| {
            let t = at(time).timestamp_millis();
            format!(r#"{{"t":{t},"org":"{org}","u":{{"fh":{pct},"sd":{pct}}}}}"#)
        };
        let history = tmp.0.join("plan-usage-history.json");
        let write = |samples: &[String]| {
            fs::write(
                &history,
                format!(r#"{{"version":2,"samples":[{}]}}"#, samples.join(",")),
            )
            .unwrap();
        };
        let a = vec![
            sample("a", "08:00", 0),
            sample("a", "08:15", 2),
            sample("a", "09:15", 4),
        ];
        write(&a);
        let mut store = UsageStore::new(Roots::from(tmp.0.join("projects")))
            .with_quota_files(vec![history.clone()]);
        store.open_archive(tmp.0.join("config"));
        store.refresh();
        let block = |data: &UsageData| {
            let block = data.active_block.as_ref().unwrap();
            (block.start_time, block.end_time, block.usage.message_count)
        };
        assert_eq!(
            block(&store.snapshot(at("09:30"))),
            (at("08:00"), at("13:00"), 2)
        );

        // Signed in to b at 10:00: 2 points per $4 response.
        let b = [
            sample("b", "10:00", 0),
            sample("b", "10:15", 2),
            sample("b", "10:30", 4),
        ];
        write(&[a.clone(), b.to_vec()].concat());
        append(&transcript, &lines(&["10:05", "10:20", "10:35"]));
        store.refresh();
        let data = store.snapshot(at("10:40"));
        assert_eq!(block(&data), (at("10:00"), at("15:00"), 3));
        let five_hour = data.quota.unwrap().five_hour.unwrap();
        assert_eq!(five_hour.resets_at, Some(at("15:00")));
        // 4 % at 10:30 plus the $4 since at b's 0.5 % per dollar.
        assert!((five_hour.estimate_percent - 6.0).abs() < 1e-9);

        // Desktop forgets a's samples; the archive still knows the switch.
        write(&b);
        let data = store.snapshot(at("10:40"));
        assert_eq!(block(&data), (at("10:00"), at("15:00"), 3));
        // The totals still count both accounts' requests.
        assert_eq!(data.total.message_count, 5);
    }

    fn at_utc(ts: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn the_archive_keeps_usage_after_its_transcripts_are_deleted() {
        let tmp = TempDir::new("archive-store");
        let logs = tmp.0.join("projects");
        let config = tmp.0.join("config");
        let (alpha, beta) = (logs.join("-work-alpha"), logs.join("-work-beta"));
        for dir in [&alpha, &beta] {
            fs::create_dir_all(dir).unwrap();
        }
        // Days that have finished in any zone at `now`.
        fs::write(
            alpha.join("s1.jsonl"),
            format!(
                "{}\n{}\n",
                line("m1", "2026-10-17T10:00:00Z", "/work/alpha", 3_000_000),
                line("m2", "2026-10-17T10:30:00Z", "/work/alpha", 1_000_000)
            ),
        )
        .unwrap();
        fs::write(
            beta.join("s2.jsonl"),
            format!(
                "{}\n",
                line("m3", "2026-10-18T09:00:00Z", "/work/beta", 2_000_000)
            ),
        )
        .unwrap();
        let now = at_utc("2026-10-19T12:00:00Z");
        let mut store = UsageStore::new(Roots::from(logs.clone()));
        store.open_archive(config.clone());
        store.refresh();
        let before = store.snapshot(now);
        assert_eq!(before.total.input_tokens, 6_000_000);
        let status = before.archive.as_ref().unwrap();
        assert_eq!((status.blocks, status.restored_responses), (2, 0));
        let read = |name| fs::read_to_string(config.join(name)).unwrap();
        let (daily, blocks) = (read(archive::DAILY_FILE), read(archive::BLOCKS_FILE));
        for text in [&daily, &blocks] {
            for private in ["\"s1\"", "\"m1\"", "req_m1", "s1.jsonl"] {
                assert!(!text.contains(private), "{private} in {text}");
            }
        }
        assert!(
            daily.contains("/work/alpha"),
            "names outlive the transcripts"
        );

        // Claude Code cleans up alpha's transcript.
        fs::remove_file(alpha.join("s1.jsonl")).unwrap();
        assert!(store.refresh());
        let after = store.snapshot(now);
        assert_eq!(
            (after.total.input_tokens, after.total.message_count),
            (6_000_000, 3)
        );
        assert!((after.total.cost_usd - before.total.cost_usd).abs() < 1e-9);
        let row = after
            .projects
            .iter()
            .find(|p| p.id == "-work-alpha")
            .unwrap();
        assert_eq!(
            (row.name.as_str(), row.path.as_deref(), row.session_count),
            ("alpha", Some("/work/alpha"), 1)
        );
        assert_eq!(row.last_activity, at_utc("2026-10-17T10:30:00Z"));
        assert_eq!(after.peak_block.unwrap().usage.input_tokens, 4_000_000);
        assert_eq!(after.archive.unwrap().restored_responses, 2);
        let summary = store.archive_summary(now);
        assert_eq!(
            (
                summary.restored.input_tokens,
                summary.restored.message_count
            ),
            (4_000_000, 2)
        );
        let blocks: Vec<_> = summary
            .blocks
            .iter()
            .map(|b| (b.total_tokens, b.message_count))
            .collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].1, 2,
            "the archived block stands in for its responses"
        );
        assert_eq!(summary.blocks[0].opened_at, at_utc("2026-10-17T10:00:00Z"));
        let files: Vec<_> = summary.files.iter().map(|f| f.name.as_str()).collect();
        assert!(files.contains(&archive::DAILY_FILE) && files.contains(&archive::BLOCKS_FILE));
        assert!(summary.reveal_path.unwrap().ends_with(archive::DAILY_FILE));

        // The next launch, with no transcripts left at all.
        fs::remove_file(beta.join("s2.jsonl")).unwrap();
        let mut fresh = UsageStore::new(Roots::from(logs));
        fresh.open_archive(config);
        fresh.refresh();
        let restored = fresh.snapshot(now);
        assert_eq!(
            (restored.total.input_tokens, restored.total.message_count),
            (6_000_000, 3)
        );
        let daily_tokens: u64 = restored.daily.iter().map(|d| d.usage.input_tokens).sum();
        assert_eq!(daily_tokens, 6_000_000);
        assert_eq!(restored.models[0].model, "claude-opus-5-5");
        assert_eq!(restored.peak_block.unwrap().usage.input_tokens, 4_000_000);
    }

    #[test]
    fn partial_cross_day_sessions_stay_unique_after_rewrites_and_restarts() {
        let tmp = TempDir::new("archive-partial-sessions");
        let logs = tmp.0.join("projects");
        let project = logs.join("-work-alpha");
        fs::create_dir_all(&project).unwrap();
        let config = tmp.0.join("config");
        let path = project.join("s1.jsonl");
        let early = line("m1", "2026-10-14T10:00:00Z", "/work/alpha", 100);
        let later = line("m2", "2026-10-16T10:00:00Z", "/work/alpha", 200);
        fs::write(&path, format!("{early}\n{later}\n")).unwrap();
        let now = at_utc("2026-10-20T12:00:00Z");
        let mut store = UsageStore::new(Roots::from(logs.clone()));
        store.open_archive(config.clone());
        store.refresh();
        let before = store.snapshot(now);
        assert_eq!(before.projects[0].session_count, 1);
        assert_eq!(before.total.input_tokens, 300);

        fs::write(&path, format!("{later}\n")).unwrap();
        assert!(store.refresh());
        let partial = store.snapshot(now);
        assert_eq!(partial.projects[0].session_count, 1);
        assert_eq!(partial.total.input_tokens, 300);
        let mut restarted = UsageStore::new(Roots::from(logs.clone()));
        restarted.open_archive(config.clone());
        restarted.refresh();
        assert_eq!(restarted.snapshot(now).projects[0].session_count, 1);

        // A continuation remains the same session; a different id adds one.
        let continued = line("m3", "2026-10-18T10:00:00Z", "/work/alpha", 300);
        let other = line("m4", "2026-10-18T11:00:00Z", "/work/alpha", 400)
            .replace("\"sessionId\":\"s1\"", "\"sessionId\":\"s2\"");
        append(&path, &format!("{continued}\n{other}\n"));
        assert!(restarted.refresh());
        let added = restarted.snapshot(now);
        assert_eq!(added.projects[0].session_count, 2);
        assert_eq!(added.total.input_tokens, 1000);

        fs::remove_file(&path).unwrap();
        let mut gone = UsageStore::new(Roots::from(logs));
        gone.open_archive(config);
        gone.refresh();
        let restored = gone.snapshot(now);
        assert_eq!(restored.projects[0].session_count, 2);
        assert_eq!(restored.total.input_tokens, 1000);
    }

    #[test]
    fn weekly_readings_outlive_desktops_own_log() {
        let tmp = TempDir::new("archive-weekly");
        let logs = tmp.0.join("projects").join("-work-alpha");
        fs::create_dir_all(&logs).unwrap();
        let at = |time: &str| at_utc(&format!("2026-10-18T{time}:00Z"));
        // $4 a response, 2 points each, after a reset at 10:15.
        let lines: Vec<String> = ["10:20", "10:35", "10:50"]
            .iter()
            .enumerate()
            .map(|(i, time)| {
                line(
                    &format!("m{i}"),
                    &format!("2026-10-18T{time}:00Z"),
                    "/work/alpha",
                    1_000_000,
                )
            })
            .collect();
        fs::write(logs.join("s.jsonl"), lines.join("\n") + "\n").unwrap();
        let samples: Vec<String> = [
            ("10:00", 10),
            ("10:15", 1),
            ("10:30", 3),
            ("10:45", 5),
            ("11:00", 7),
        ]
        .iter()
        .map(|(time, pct)| {
            let t = at(time).timestamp_millis();
            format!(r#"{{"t":{t},"org":"org-secret","u":{{"fh":{pct},"sd":{pct}}}}}"#)
        })
        .collect();
        let history = tmp.0.join("plan-usage-history.json");
        let write = |samples: &[String]| {
            fs::write(
                &history,
                format!(r#"{{"version":2,"samples":[{}]}}"#, samples.join(",")),
            )
            .unwrap();
        };
        write(&samples);
        let config = tmp.0.join("config");
        let mut store = UsageStore::new(Roots::from(tmp.0.join("projects")))
            .with_quota_files(vec![history.clone()]);
        store.open_archive(config.clone());
        store.refresh();
        let week = store.snapshot(at("12:00")).quota.unwrap().week.unwrap();
        assert_eq!(
            (week.period_start, week.reset_observed),
            (at("10:00"), true)
        );

        // Desktop keeps only its newest sample: the archive still has the reset.
        write(&samples[4..]);
        let week = store.snapshot(at("12:00")).quota.unwrap().week.unwrap();
        assert_eq!(
            (week.period_start, week.reset_observed),
            (at("10:00"), true)
        );
        assert_eq!(week.projects.len(), 1);
        assert!((week.projects[0].percent - 6.0).abs() < 1e-9);

        let text = fs::read_to_string(config.join(archive::WEEKLY_FILE)).unwrap();
        assert!(!text.contains("org-secret"), "only a hash of the account");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let account = &json["accounts"][0];
        let readings = account["readings"].as_array().unwrap();
        assert_eq!(readings.len(), 5);
        assert!(readings.iter().all(|r| r["source"] == "official"));
        assert_eq!(account["resets"][0]["source"], "official");
        assert_eq!(account["resets"][0]["toPercent"], 1.0);
        let fits = account["calibration"].as_array().unwrap();
        assert!(!fits.is_empty() && fits.iter().all(|c| c["source"] == "estimate"));
    }

    #[test]
    fn the_quota_fit_is_kept_and_the_archive_left_alone_while_only_the_clock_moves() {
        let tmp = TempDir::new("archive-quiet-ticks");
        let logs = tmp.0.join("projects").join("-work-alpha");
        fs::create_dir_all(&logs).unwrap();
        let transcript = logs.join("s.jsonl");
        let base = at_utc("2026-10-18T10:00:00Z");
        let minutes = |m: i64| base + TimeDelta::minutes(m);
        // $4 a response between Desktop's readings, which are stamped to the
        // millisecond and rise unevenly, so the recency weights shape the fit.
        let lines: Vec<String> = (0..4)
            .map(|i| {
                let ts = minutes(15 * i + 5).to_rfc3339();
                line(&format!("m{i}"), &ts, "/work/alpha", 1_000_000)
            })
            .collect();
        fs::write(&transcript, lines.join("\n") + "\n").unwrap();
        let mut samples: Vec<String> = [(123, 1), (457, 3), (789, 9), (12, 11), (345, 17)]
            .iter()
            .zip(0..)
            .map(|((ms, pct), i)| {
                let t = (minutes(15 * i) + TimeDelta::milliseconds(*ms)).timestamp_millis();
                format!(r#"{{"t":{t},"org":"a","u":{{"fh":{pct},"sd":{pct}}}}}"#)
            })
            .collect();
        let history = tmp.0.join("plan-usage-history.json");
        let write = |samples: &[String]| {
            fs::write(
                &history,
                format!(r#"{{"version":2,"samples":[{}]}}"#, samples.join(",")),
            )
            .unwrap();
        };
        write(&samples);
        let config = tmp.0.join("config");
        let mut store = UsageStore::new(Roots::from(tmp.0.join("projects")))
            .with_quota_files(vec![history.clone()]);
        store.open_archive(config.clone());
        store.refresh();
        let now = minutes(70);
        assert!(store.snapshot(now).quota.is_some());
        let weekly = config.join(archive::WEEKLY_FILE);
        assert!(weekly.exists());
        let fit = store.quota_fit.as_ref().unwrap();
        let (spend, fitted) = (Arc::clone(&fit.spend), fit.fitted);
        assert!(fitted.five_hour.is_some() && fitted.seven_day.is_some());

        // Snapshots a fraction of a second or more apart reuse the spend and
        // the fit, and the archive is not written again.
        fs::remove_file(&weekly).unwrap();
        for later in [1, 250, 999, 1_500, 17_321] {
            store.snapshot(now + TimeDelta::milliseconds(later));
            let fit = store.quota_fit.as_ref().unwrap();
            assert!(Arc::ptr_eq(&fit.spend, &spend), "{later} ms later");
            assert_eq!(fit.fitted, fitted, "{later} ms later");
            assert!(!weekly.exists(), "rewritten {later} ms later");
        }

        // A new reading is fitted on the same spend.
        let t = (minutes(75) + TimeDelta::milliseconds(678)).timestamp_millis();
        samples.push(format!(r#"{{"t":{t},"org":"a","u":{{"fh":19,"sd":19}}}}"#));
        write(&samples);
        store.snapshot(minutes(76));
        let fit = store.quota_fit.as_ref().unwrap();
        assert!(Arc::ptr_eq(&fit.spend, &spend));
        assert_eq!(fit.samples.len(), 6);
        assert!(weekly.exists(), "the new reading is archived");

        // New usage collects the spend again.
        let ts = minutes(77).to_rfc3339();
        append(
            &transcript,
            &format!("{}\n", line("m9", &ts, "/work/alpha", 1_000)),
        );
        assert!(store.refresh());
        store.snapshot(minutes(78));
        assert!(!Arc::ptr_eq(
            &store.quota_fit.as_ref().unwrap().spend,
            &spend
        ));
    }

    #[test]
    fn new_prices_reprice_what_was_read_without_reading_it_again() {
        let tmp = TempDir::new("pricing-in-memory");
        let logs = tmp.0.join("projects");
        fs::create_dir_all(&logs).unwrap();
        let file = logs.join("s.jsonl");
        let record = |input| format!("{}\n", line("m", "2026-10-18T10:00:00Z", "/p", input));
        fs::write(&file, record(1_000_000)).unwrap();
        let modified = file.metadata().unwrap().modified().unwrap();
        let prices = tmp.0.join("pricing.json");
        let mut store = UsageStore::new(Roots::from(logs)).with_pricing_file(Some(prices.clone()));
        store.refresh();
        assert!((store.snapshot(Utc::now()).total.cost_usd - 4.0).abs() < 1e-9);

        // A change no metadata shows: only reading the transcript would see it.
        fs::write(&file, record(9_000_000)).unwrap();
        set_modified(&file, modified);
        fs::write(
            &prices,
            r#"{ "claude-opus-5-5": { "input": 9, "output": 45 } }"#,
        )
        .unwrap();
        let generation = store.generation();
        assert!(store.refresh());
        assert!(store.generation() > generation);
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.input_tokens, 1_000_000, "not read again");
        assert!((data.total.cost_usd - 9.0).abs() < 1e-9);
        // A rescan reads it, at the new prices.
        assert!(store.rescan());
        assert!((store.snapshot(Utc::now()).total.cost_usd - 81.0).abs() < 1e-9);
    }

    #[cfg(windows)]
    #[test]
    fn new_prices_reprice_a_transcript_that_cannot_be_read() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new("pricing-locked-transcript");
        let logs = tmp.0.join("projects");
        fs::create_dir_all(&logs).unwrap();
        let file = logs.join("s.jsonl");
        fs::write(
            &file,
            format!("{}\n", line("m", "2026-10-18T10:00:00Z", "/p", 1_000_000)),
        )
        .unwrap();
        let prices = tmp.0.join("pricing.json");
        let config = tmp.0.join("config");
        let mut store = UsageStore::new(Roots::from(logs)).with_pricing_file(Some(prices.clone()));
        store.open_archive(config.clone());
        store.refresh();
        // A day that has finished in any zone, so it is archived.
        let now = at_utc("2026-10-20T12:00:00Z");
        assert!((store.snapshot(now).total.cost_usd - 4.0).abs() < 1e-9);

        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        fs::write(
            &prices,
            r#"{ "claude-opus-5-5": { "input": 9, "output": 45 } }"#,
        )
        .unwrap();
        assert!(store.refresh());
        let data = store.snapshot(now);
        drop(exclusive);
        assert!((data.total.cost_usd - 9.0).abs() < 1e-9);
        assert!(data.read_errors.is_empty(), "{:?}", data.read_errors);
        // The archive's costs match the prices it records.
        let daily: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config.join(archive::DAILY_FILE)).unwrap())
                .unwrap();
        assert_eq!(daily["prices"]["claude-opus-5-5"]["input"], 9.0);
        let cost = daily["days"][0]["usage"][0]["costUsd"].as_f64().unwrap();
        assert!((cost - 9.0).abs() < 1e-9, "{cost}");
    }

    #[cfg(windows)]
    #[test]
    fn a_price_file_that_could_not_be_read_is_retried() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new("pricing-locked");
        let logs = tmp.0.join("projects");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("s.jsonl"),
            format!("{}\n", line("m", "2026-10-18T10:00:00Z", "/p", 1_000_000)),
        )
        .unwrap();
        let prices = tmp.0.join("pricing.json");
        fs::write(
            &prices,
            r#"{ "claude-opus-5-5": { "input": 9, "output": 45 } }"#,
        )
        .unwrap();
        let mut store = UsageStore::new(Roots::from(logs)).with_pricing_file(Some(prices.clone()));
        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&prices)
            .unwrap();
        store.refresh();
        let locked = store.snapshot(Utc::now());
        assert!(locked.pricing_error.is_some());
        assert!(
            (locked.total.cost_usd - 4.0).abs() < 1e-9,
            "built-in price meanwhile"
        );
        drop(exclusive);
        // The metadata is unchanged, so only a retry picks the prices up.
        assert!(store.refresh());
        let data = store.snapshot(Utc::now());
        assert_eq!((data.custom_prices, data.pricing_error), (1, None));
        assert!((data.total.cost_usd - 9.0).abs() < 1e-9);
    }

    #[test]
    fn validation_catches_silent_rewrites_and_rejects_obsolete_results() {
        let tmp = TempDir::new("validation-incarnation");
        let file = tmp.0.join("s.jsonl");
        let record = |tokens| format!("{}\n", line("same", "2026-10-18T10:00:00Z", "/p", tokens));
        fs::write(&file, record(10)).unwrap();
        let modified = file.metadata().unwrap().modified().unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        let job = store.validation_jobs(true).pop().unwrap();
        // Appending after capturing a job must not starve validation.
        append(
            &file,
            &format!("{}\n", line("new", "2026-10-18T10:01:00Z", "/p", 1)),
        );
        store.refresh();
        let mut contents = fs::read_to_string(&file).unwrap();
        contents = contents.replace("\"input_tokens\":10", "\"input_tokens\":90");
        fs::write(&file, contents).unwrap();
        set_modified(&file, modified);
        assert!(store.apply_validation(job.check()));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 91);
        let obsolete = store.validation_jobs(true).pop().unwrap();
        fs::write(&file, record(50)).unwrap();
        store.rescan();
        let generation = store.generation();
        assert!(!store.apply_validation(obsolete.check()));
        assert_eq!(store.generation(), generation);
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 50);
    }

    #[test]
    fn routine_validation_audits_only_grown_transcripts() {
        let tmp = TempDir::new("validation-scope");
        let (grown, idle) = (tmp.0.join("grown.jsonl"), tmp.0.join("idle.jsonl"));
        for (path, id) in [(&grown, "g"), (&idle, "i")] {
            fs::write(
                path,
                format!("{}\n", line(id, "2026-10-18T10:00:00Z", "/p", 1)),
            )
            .unwrap();
        }
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        assert!(
            store.validation_jobs(false).is_empty(),
            "both were just read in full"
        );
        assert_eq!(store.validation_jobs(true).len(), 2);
        append(
            &grown,
            &format!("{}\n", line("g2", "2026-10-18T10:01:00Z", "/p", 1)),
        );
        store.refresh();
        let jobs = store.validation_jobs(false);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].path, grown);
        for job in jobs {
            assert!(!store.apply_validation(job.check()));
        }
        assert!(
            store.validation_jobs(false).is_empty(),
            "the audited prefix is remembered"
        );
    }

    #[test]
    fn directory_events_check_stamps_and_file_events_verify_contents() {
        let tmp = TempDir::new("directory-events");
        let project = tmp.0.join("project");
        fs::create_dir_all(&project).unwrap();
        let (quiet, busy) = (project.join("quiet.jsonl"), project.join("busy.jsonl"));
        let record = |id, tokens| format!("{}\n", line(id, "2026-10-18T10:00:00Z", "/p", tokens));
        fs::write(&quiet, record("q", 10)).unwrap();
        fs::write(&busy, record("b", 20)).unwrap();
        let modified = quiet.metadata().unwrap().modified().unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        // Same length and timestamp: only a full read could tell.
        fs::write(&quiet, record("q", 90)).unwrap();
        set_modified(&quiet, modified);
        append(&busy, &record("b2", 1));
        let new = project.join("new.jsonl");
        fs::write(&new, record("n", 2)).unwrap();
        // A new transcript also notifies its folder (Windows). The other files
        // are checked by stamp: the append is read, the quiet file is not reread.
        assert!(store.refresh_paths(&HashSet::from([project.clone(), new.clone()])));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10 + 21 + 2);
        // An event naming the file still verifies its whole prefix.
        assert!(store.refresh_paths(&HashSet::from([quiet.clone()])));
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 90 + 21 + 2);
    }

    #[test]
    fn parent_link_index_is_bounded_and_still_resolves_requests() {
        let tmp = TempDir::new("bounded-links");
        let file = tmp.0.join("s.jsonl");
        let mut lines: Vec<String> = (0..3 * MAX_RECORD_LINKS)
            .map(|i| {
                let parent = i.checked_sub(1).map_or("null".to_owned(), |p| format!("\"u{p}\""));
                format!(r#"{{"type":"user","uuid":"u{i}","parentUuid":{parent},"sessionId":"s1","timestamp":"2026-10-18T10:00:00Z"}}"#)
            })
            .collect();
        let response = |id, ts, parent: &str| {
            let mut record: serde_json::Value =
                serde_json::from_str(&line(id, ts, "/p", 1)).unwrap();
            record["uuid"] = format!("uuid-{id}").into();
            record["parentUuid"] = parent.into();
            record.to_string()
        };
        let last = 3 * MAX_RECORD_LINKS - 1;
        lines.push(format!(r#"{{"type":"user","uuid":"prompt","parentUuid":"u{last}","sessionId":"s1","timestamp":"2026-10-18T11:00:00Z"}}"#));
        lines.push(response("answer", "2026-10-18T11:00:05Z", "prompt"));
        // A parent that fell out of the window: the response's own time.
        lines.push(response("orphan", "2026-10-18T11:00:09Z", "u0"));
        fs::write(&file, lines.join("\n") + "\n").unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        store.refresh();
        let state = &store.files[&file];
        assert_eq!(state.records.len(), MAX_RECORD_LINKS);
        assert_eq!(state.record_order.len(), MAX_RECORD_LINKS);
        let at = |ts| DateTime::parse_from_rfc3339(ts).unwrap();
        assert_eq!(
            state.entries["answer"].requested_at,
            at("2026-10-18T11:00:00Z")
        );
        assert_eq!(
            state.entries["orphan"].requested_at,
            at("2026-10-18T11:00:09Z")
        );
    }

    #[cfg(windows)]
    #[test]
    fn read_errors_preserve_cached_usage_and_clear_on_recovery() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = TempDir::new("read-errors");
        let file = tmp.0.join("s.jsonl");
        fs::write(
            &file,
            format!("{}\n", line("m", "2026-10-18T10:00:00Z", "/p", 10)),
        )
        .unwrap();
        let mut store = UsageStore::new(Roots::from(tmp.0.clone()));
        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        store.refresh();
        let failed = store.snapshot(Utc::now());
        assert_eq!(failed.read_errors.len(), 1);
        assert_eq!(failed.total.total_tokens, 0);
        drop(exclusive);
        assert!(store.refresh());
        assert!(store.snapshot(Utc::now()).read_errors.is_empty());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10);
        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        assert!(store.rescan());
        let stale = store.snapshot(Utc::now());
        assert_eq!(stale.read_errors.len(), 1);
        assert_eq!(
            stale.total.input_tokens, 10,
            "failed rescan preserves cached data"
        );
        let generation = store.generation();
        drop(exclusive);
        assert!(store.refresh());
        assert!(
            store.generation() > generation,
            "health recovery must be published even without new usage"
        );
        assert!(store.snapshot(Utc::now()).read_errors.is_empty());
        assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 10);
    }

    #[test]
    fn missing_root_yields_empty_data() {
        let mut store = UsageStore::new(Roots::from(PathBuf::from(
            "/definitely/not/here/cc-deskmeter",
        )));
        assert!(!store.refresh());
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.total_tokens, 0);
        assert!(data.daily.is_empty() && data.active_block.is_none());
    }

    #[test]
    fn encodes_cwd_like_claude_code() {
        assert!(encodes_to(
            r"D:\Pycharm Project\sucess_12306",
            "D--Pycharm-Project-sucess-12306"
        ));
        assert!(encodes_to("/Users/me/my.app", "-Users-me-my-app"));
        assert!(!encodes_to("/Users/me/my.app/src", "-Users-me-my-app"));
    }

    /// `cargo test -- --ignored --nocapture scan_real_logs`
    /// (set `CC_DESKMETER_ROOT` to scan a copy instead of the discovered roots).
    #[test]
    #[ignore = "reads the real Claude Code transcripts"]
    fn scan_real_logs() {
        let roots = std::env::var_os("CC_DESKMETER_ROOT")
            .map_or_else(Roots::discover, |root| Roots::from(PathBuf::from(root)));
        let mut store = UsageStore::new(roots).with_quota_files(quota::default_paths());
        // A throwaway archive, to time building it from every transcript.
        let archive = TempDir::new("real-archive");
        store.open_archive(archive.0.clone());
        let started = std::time::Instant::now();
        store.refresh();
        let scan = started.elapsed();
        let started = std::time::Instant::now();
        let data = store.snapshot(Utc::now());
        let first = started.elapsed();
        let started = std::time::Instant::now();
        store.snapshot(Utc::now());
        let tick = started.elapsed();
        println!("{}", serde_json::to_string_pretty(&data).unwrap());
        println!("initial scan: {scan:?}; first snapshot (archives): {first:?}; next: {tick:?}");
        for name in [
            archive::DAILY_FILE,
            archive::BLOCKS_FILE,
            archive::WEEKLY_FILE,
        ] {
            let size = fs::metadata(archive.0.join(name)).map_or(0, |meta| meta.len());
            println!("{name}: {size} bytes");
        }
    }
}
