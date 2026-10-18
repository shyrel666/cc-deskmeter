//! Incremental reader for `~/.claude/projects/**/*.jsonl`.
//!
//! Appends check bounded head/tail samples and read only new bytes. Full prefix
//! checks run off the store lock in the background; manual rescans reparse all
//! files. Truncations, replacements and detected rewrites rebuild their index.

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
use super::parser::{self, ParsedUsage, RecordContext};
use super::pricing;
use super::types::{ReadError, UsageData, UsageEntry};

/// Parent links kept per transcript. A response's chain reaches back only to
/// the prompt or tool result that sent it (at most 4 records in real logs);
/// an evicted parent falls back to the response's own timestamp.
const MAX_RECORD_LINKS: usize = 1024;

/// Locks the store, recovering from a poisoned mutex (its data stays consistent).
pub fn lock(store: &Mutex<UsageStore>) -> MutexGuard<'_, UsageStore> {
    store.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct UsageStore {
    root: Option<PathBuf>,
    files: HashMap<PathBuf, FileState>,
    /// Bumped whenever usage, file metadata displayed by the UI or read health changes.
    generation: u64,
    snapshot_id: u64,
    history: History,
    project_meta: HashMap<Arc<str>, ProjectMeta>,
    read_errors: BTreeMap<PathBuf, String>,
}

impl UsageStore {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            files: HashMap::new(),
            generation: 0,
            snapshot_id: 0,
            history: History::default(),
            project_meta: HashMap::new(),
            read_errors: BTreeMap::new(),
        }
    }

    /// `~/.claude/projects` on every platform.
    pub fn default_root() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".claude").join("projects"))
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Picks up new lines, new files and deleted files. Returns whether any
    /// usage entry was added or removed.
    pub fn refresh(&mut self) -> bool {
        self.refresh_inner(false, &HashSet::new())
    }

    /// Inspect only notified files/subtrees, including removals. Equal-length
    /// writes are verified even if the writer preserved the modification time.
    pub fn refresh_paths(&mut self, paths: &HashSet<PathBuf>) -> bool {
        self.refresh_inner(false, paths)
    }

    pub fn rescan(&mut self) -> bool {
        self.refresh_inner(true, &HashSet::new())
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
        let Some(root) = self.root.clone() else {
            return false;
        };
        let mut candidates: Vec<_> = if paths.is_empty() {
            vec![root.clone()]
        } else {
            paths
                .iter()
                .filter(|p| p.starts_with(&root))
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

        for item in scopes.iter().flat_map(WalkDir::new) {
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
                        eprintln!("claude-pulse: failed to enumerate transcripts: {err}");
                        self.read_errors
                            .insert(err.path().unwrap_or(&root).to_path_buf(), err.to_string());
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
            let (added, reset) = self.ingest(&root, &path, &meta, force, notified);
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
            self.history = History::default();
            for entry in self.files.values().flat_map(|state| state.entries.values()) {
                self.history.insert(Arc::clone(entry), &Local);
            }
        }
        if changed {
            self.generation += 1;
            self.project_meta = self.project_meta();
        }
        changed
    }

    pub fn snapshot(&mut self, now: DateTime<Utc>) -> UsageData {
        let mut data = self.history.snapshot(&self.project_meta, now, &Local);
        self.snapshot_id += 1;
        data.snapshot_id = self.snapshot_id;
        data.data_dir = self.root.as_ref().map(|root| root.display().to_string());
        data.file_count = self.files.len();
        data.read_errors = self
            .read_errors
            .iter()
            .map(|(path, message)| ReadError {
                path: path.display().to_string(),
                message: message.clone(),
            })
            .collect();
        data
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
                replacement.read_new_lines(&mut file, &mut |_| {}, &mut replacement_changed)?;
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
            eprintln!("claude-pulse: failed to read {}: {err}", path.display());
            self.read_errors.insert(path.to_path_buf(), err.to_string());
        }
        (changed, reset)
    }

    /// Display name and path per project folder. Prefers a `cwd` that maps back
    /// to the folder name exactly, then the shortest (closest to the root).
    fn project_meta(&self) -> HashMap<Arc<str>, ProjectMeta> {
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
            *changed |= self.ingest_line(&line, on_entry);
            self.checksum.update(&line);
            self.offset += read as u64;
        }
        Ok(())
    }

    /// `self.offset` must still point at the start of `line`.
    fn ingest_line(&mut self, line: &[u8], on_entry: &mut impl FnMut(Arc<UsageEntry>)) -> bool {
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
        let entry = self.to_entry(parsed);
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

    fn to_entry(&self, parsed: ParsedUsage) -> UsageEntry {
        let request_time = self.request_time(&parsed);
        let rates = pricing::rates_for(&parsed.model);
        let cost_usd = rates.map_or(0.0, |rates| {
            pricing::cost_usd(&rates, &parsed.tokens, parsed.cache_write_1h, parsed.fast)
        });
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

        UsageEntry {
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
            cost_usd,
            priced: rates.is_some(),
        }
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
                std::env::temp_dir().join(format!("claude-pulse-{name}-{}", std::process::id()));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
            let mut store = UsageStore::new(Some(tmp.0.clone()));
            store.refresh();
            assert_eq!(store.snapshot(Utc::now()).total.input_tokens, 100);
        }
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        assert_eq!(block.start_time.to_rfc3339(), "2026-10-18T01:51:00+00:00");
        assert_eq!(block.end_time.to_rfc3339(), "2026-10-18T06:51:00+00:00");
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
            assert_eq!(state.to_entry(parsed).requested_at, timestamp);
        }
    }

    #[test]
    fn snapshot_order_does_not_depend_on_clock_or_usage_changes() {
        let mut store = UsageStore::new(None);
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
    fn validation_catches_silent_rewrites_and_rejects_obsolete_results() {
        let tmp = TempDir::new("validation-incarnation");
        let file = tmp.0.join("s.jsonl");
        let record = |tokens| format!("{}\n", line("same", "2026-10-18T10:00:00Z", "/p", tokens));
        fs::write(&file, record(10)).unwrap();
        let modified = file.metadata().unwrap().modified().unwrap();
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
            fs::write(path, format!("{}\n", line(id, "2026-10-18T10:00:00Z", "/p", 1))).unwrap();
        }
        let mut store = UsageStore::new(Some(tmp.0.clone()));
        store.refresh();
        assert!(store.validation_jobs(false).is_empty(), "both were just read in full");
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
        assert!(store.validation_jobs(false).is_empty(), "the audited prefix is remembered");
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
        store.refresh();
        let state = &store.files[&file];
        assert_eq!(state.records.len(), MAX_RECORD_LINKS);
        assert_eq!(state.record_order.len(), MAX_RECORD_LINKS);
        let at = |ts| DateTime::parse_from_rfc3339(ts).unwrap();
        assert_eq!(state.entries["answer"].requested_at, at("2026-10-18T11:00:00Z"));
        assert_eq!(state.entries["orphan"].requested_at, at("2026-10-18T11:00:09Z"));
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
        let mut store = UsageStore::new(Some(tmp.0.clone()));
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
        let mut store = UsageStore::new(Some(PathBuf::from("/definitely/not/here/claude-pulse")));
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
    /// (set `CLAUDE_PULSE_ROOT` to scan a copy instead of `~/.claude/projects`).
    #[test]
    #[ignore = "reads the real ~/.claude/projects"]
    fn scan_real_logs() {
        let root = std::env::var_os("CLAUDE_PULSE_ROOT")
            .map(PathBuf::from)
            .or_else(UsageStore::default_root);
        let mut store = UsageStore::new(root);
        let started = std::time::Instant::now();
        store.refresh();
        let scan = started.elapsed();
        let data = store.snapshot(Utc::now());
        println!("{}", serde_json::to_string_pretty(&data).unwrap());
        println!("initial scan: {scan:?}");
    }
}
