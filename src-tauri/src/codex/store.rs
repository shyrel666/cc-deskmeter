//! Incremental reader for every rollout under the Codex home.
//!
//! Plain rollouts grow by appended lines and are read on from where the last
//! read stopped, once their head and the bytes before that point check out;
//! a rewrite, truncation or replacement reads the file again. Compressed
//! rollouts never change once written and are read whole whenever their
//! metadata does (a damaged one keeps its error until then, and is not
//! decoded on every refresh). A rollout found more than once (compressed beside its plain
//! copy, or archived as well as live) counts once, preferring a successfully
//! read copy, then its live, plain copy.
//! Files are read on a few threads at once: the first scan decompresses and
//! parses every rollout (500 MB of JSON on the author's machine). The local
//! `pricing.json` is read here too, as the usage store reads it for Claude.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::SystemTime;

use chrono::{DateTime, Local, Utc};
use serde::Deserialize;
use serde::de::IgnoredAny;
use walkdir::WalkDir;

use super::ledger::Ledger;
use super::parser::FileLog;
use super::paths::{CodexHome, Encoding, rollout_key};
use super::reader::ZstReader;
use super::report::{self, Titles};
use super::signin::SignIns;
use super::types::{CodexData, CodexHistory};
use crate::usage::{Pricing, ReadError};

/// Bytes of a plain rollout's head and of the tail before the read offset
/// compared before reading on, to catch a rewrite in place.
const ANCHOR_BYTES: u64 = 4096;
/// Threads reading rollouts at once. Each holds one decoder window (a few
/// MB) and the lines it parses; more add little once the disk is the limit.
const READERS: usize = 4;

/// Locks the store, recovering from a poisoned mutex (its data stays consistent).
pub fn lock(store: &Mutex<CodexStore>) -> MutexGuard<'_, CodexStore> {
    store.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct CodexStore {
    home: CodexHome,
    files: HashMap<PathBuf, FileState>,
    titles: Titles,
    /// Metadata of `session_index.jsonl` at its last read, `Some(None)` while
    /// it is absent; `None` reads it on the next refresh.
    titles_stamp: Option<Option<FileStamp>>,
    read_errors: BTreeMap<PathBuf, String>,
    /// The local overrides of the built-in prices (see `pricing`).
    pricing: Pricing,
    /// Optional local price overrides; `None` uses the built-in table only.
    pricing_file: Option<PathBuf>,
    /// Metadata of `pricing_file` at its last completed read, `Some(None)`
    /// while it is absent. `None` makes the next refresh read it.
    pricing_stamp: Option<Option<FileStamp>>,
    pricing_error: Option<String>,
    /// Bumped whenever usage, titles, prices or read health change.
    generation: u64,
    snapshot_id: u64,
    /// The ledger as of `generation`.
    ledger: Option<(u64, Arc<Ledger>)>,
    /// Who was signed in when, to tell whose limits a reading shows.
    signins: SignIns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

impl From<&Metadata> for FileStamp {
    fn from(meta: &Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
        }
    }
}

struct FileState {
    key: String,
    /// Index into `CodexHome::roots` (live before archived).
    root: usize,
    encoding: Encoding,
    stamp: Option<FileStamp>,
    /// Plain rollouts: bytes read so far, always at a line boundary.
    offset: u64,
    /// Plain rollouts: checksums of the head and of the bytes before `offset`.
    anchors: Option<(u32, u32)>,
    log: FileLog,
    failed: bool,
    /// A compressed rollout whose content failed to decode: its metadata
    /// then, and the error. Until that metadata changes it would fail again.
    damaged: Option<(FileStamp, String)>,
}

impl FileState {
    fn new(key: String, root: usize, encoding: Encoding) -> Self {
        Self {
            key,
            root,
            encoding,
            stamp: None,
            offset: 0,
            anchors: None,
            log: FileLog::default(),
            failed: false,
            damaged: None,
        }
    }

    /// Whether the last read completed successfully.
    fn readable(&self) -> bool {
        !self.failed && self.stamp.is_some()
    }
}

impl CodexStore {
    pub fn new(home: CodexHome) -> Self {
        Self {
            signins: SignIns::new(&home.dir),
            home,
            files: HashMap::new(),
            titles: Titles::new(),
            titles_stamp: None,
            read_errors: BTreeMap::new(),
            pricing: Pricing::default(),
            pricing_file: None,
            pricing_stamp: None,
            pricing_error: None,
            generation: 0,
            snapshot_id: 0,
            ledger: None,
        }
    }

    /// Reads local price overrides from `path`, if it exists, on every full
    /// refresh (so edits apply within one tick) and on every rescan. New
    /// prices reprice what was read already, without reading it again.
    pub fn with_pricing_file(mut self, path: Option<PathBuf>) -> Self {
        self.pricing_file = path;
        self
    }

    /// Keeps the sign-ins seen in `dir`, the app's own folder, so they
    /// still tell whose limits a reading shows after the next switch.
    pub fn open_signins(&mut self, dir: &Path) {
        self.signins.open(dir);
        self.generation += 1;
    }

    /// Folders to watch: rollouts live in both.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.home.roots().to_vec()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Picks up new lines, files and deletions, and new prices. Returns
    /// whether anything shown changed.
    pub fn refresh(&mut self) -> bool {
        let repriced = self.reload_pricing();
        let signed = self.observe_signin();
        self.refresh_inner(false, &HashSet::new()) || repriced || signed
    }

    /// Inspects only notified files or folders (removals included).
    pub fn refresh_paths(&mut self, paths: &HashSet<PathBuf>) -> bool {
        let signed = self.observe_signin();
        self.refresh_inner(false, paths) || signed
    }

    /// Looks at who is signed in; a change shows whose limits are current.
    fn observe_signin(&mut self) -> bool {
        let changed = self.signins.observe(Utc::now());
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Reads every rollout again from the start.
    pub fn rescan(&mut self) -> bool {
        self.titles_stamp = None;
        let repriced = self.reload_pricing();
        let signed = self.observe_signin();
        self.refresh_inner(true, &HashSet::new()) || repriced || signed
    }

    /// Re-reads the overrides when the file appeared, changed or went away,
    /// or when the last read failed, as `UsageStore` does with the same
    /// file. Returns whether prices changed: every cost does then, as the
    /// ledger is built again. An unreadable or invalid file keeps the last
    /// valid prices.
    fn reload_pricing(&mut self) -> bool {
        let Some(path) = self.pricing_file.as_deref() else {
            return false;
        };
        let stamp = std::fs::metadata(path)
            .ok()
            .map(|meta| FileStamp::from(&meta));
        if self.pricing_stamp == Some(stamp) {
            return false;
        }
        let read = std::fs::read_to_string(path);
        // A read error can pass without touching the metadata, so it is
        // retried on the next refresh. Invalid content waits for a change.
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
        // The usage store, reading the same file, logs the error.
        if error != self.pricing_error {
            self.pricing_error = error;
            self.generation += 1;
        }
        if pricing == self.pricing {
            return false;
        }
        self.pricing = pricing;
        self.generation += 1;
        true
    }

    fn refresh_inner(&mut self, force: bool, paths: &HashSet<PathBuf>) -> bool {
        let roots = self.home.roots();
        let root_of = |path: &Path| roots.iter().position(|root| path.starts_with(root));
        let mut candidates: Vec<PathBuf> = if paths.is_empty() {
            roots.to_vec()
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
            if !scopes.iter().any(|scope| candidate.starts_with(scope)) {
                scopes.push(candidate);
            }
        }
        let affected = |path: &Path| scopes.iter().any(|scope| path.starts_with(scope));

        let previous_errors = self.read_errors.clone();
        self.read_errors.retain(|path, _| !affected(path));
        let mut changed = self.reload_titles();
        let mut seen = HashSet::new();
        let mut jobs = Vec::new();
        let mut complete = true;
        for scope in &scopes {
            let root = root_of(scope).expect("scopes lie under a root");
            for item in WalkDir::new(scope) {
                let item = match item {
                    Ok(item) => item,
                    Err(err) => {
                        // Only a complete walk proves a known file is gone; a
                        // missing folder is a real removal.
                        if err
                            .io_error()
                            .is_none_or(|err| err.kind() != io::ErrorKind::NotFound)
                        {
                            complete = false;
                            let path = err.path().unwrap_or(scope).to_path_buf();
                            self.read_errors.insert(path, err.to_string());
                        }
                        continue;
                    }
                };
                if !item.file_type().is_file() {
                    continue;
                }
                let Some((key, encoding)) = rollout_key(item.path()) else {
                    continue;
                };
                let meta = match item.metadata() {
                    Ok(meta) => meta,
                    Err(err) => {
                        complete = false;
                        self.read_errors
                            .insert(item.path().to_path_buf(), err.to_string());
                        continue;
                    }
                };
                let path = item.into_path();
                // A notified file is checked even when its metadata looks unchanged.
                let recheck = force || paths.contains(&path);
                let stamp = FileStamp::from(&meta);
                seen.insert(path.clone());
                if let Some(state) = self.files.get(&path).filter(|_| !recheck) {
                    if !state.failed && state.stamp == Some(stamp) {
                        continue;
                    }
                    if let Some((at, error)) = &state.damaged
                        && *at == stamp
                    {
                        self.read_errors.insert(path, error.clone());
                        continue;
                    }
                }
                let state = self
                    .files
                    .remove(&path)
                    .unwrap_or_else(|| FileState::new(key, root, encoding));
                jobs.push(Job { path, state, stamp });
            }
        }
        for (job, result) in read_all(jobs) {
            let Job {
                path,
                mut state,
                stamp,
            } = job;
            state.failed = result.is_err();
            state.damaged = None;
            match result {
                Ok(read) => changed |= read,
                Err(err) => {
                    eprintln!("cc-deskmeter: failed to read {}: {err}", path.display());
                    // Only damaged content lasts: an I/O error (a sharing
                    // violation, say) can pass with the metadata unchanged.
                    if state.encoding == Encoding::Zstd && err.kind() == io::ErrorKind::InvalidData
                    {
                        state.damaged = Some((stamp, err.to_string()));
                    }
                    self.read_errors.insert(path.clone(), err.to_string());
                }
            }
            self.files.insert(path, state);
        }
        if complete {
            let before = self.files.len();
            self.files
                .retain(|path, _| !affected(path) || seen.contains(path));
            changed |= self.files.len() != before;
        }
        changed |= self.read_errors != previous_errors;
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Re-reads `session_index.jsonl` when it appeared, changed or went away.
    fn reload_titles(&mut self) -> bool {
        let path = self.home.session_index();
        let stamp = std::fs::metadata(&path)
            .ok()
            .map(|meta| FileStamp::from(&meta));
        if self.titles_stamp == Some(stamp) {
            return false;
        }
        let titles = match std::fs::read(&path) {
            Ok(bytes) => parse_titles(&bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Titles::new(),
            Err(err) => {
                // Titles are cosmetic: keep the last ones and retry next time.
                eprintln!("cc-deskmeter: failed to read {}: {err}", path.display());
                return false;
            }
        };
        self.titles_stamp = Some(stamp);
        if titles == self.titles {
            return false;
        }
        self.titles = titles;
        true
    }

    /// The ledger over one copy of each rollout, rebuilt when a file or a
    /// price changed.
    fn ledger(&mut self) -> Arc<Ledger> {
        if let Some((generation, ledger)) = &self.ledger
            && *generation == self.generation
        {
            return Arc::clone(ledger);
        }
        let mut chosen: BTreeMap<&str, &FileState> = BTreeMap::new();
        for state in self.files.values() {
            let better = chosen.get(state.key.as_str()).is_none_or(|kept| {
                (!state.readable(), state.root, state.encoding)
                    < (!kept.readable(), kept.root, kept.encoding)
            });
            if better {
                chosen.insert(&state.key, state);
            }
        }
        let ledger = Arc::new(Ledger::build(
            chosen.iter().map(|(key, state)| (*key, &state.log)),
            &self.pricing,
        ));
        self.ledger = Some((self.generation, Arc::clone(&ledger)));
        ledger
    }

    fn rollout_count(&self) -> usize {
        self.files
            .values()
            .map(|state| state.key.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    pub fn snapshot(&mut self, now: DateTime<Utc>) -> CodexData {
        let ledger = self.ledger();
        let mut data = report::report(
            &ledger,
            &self.titles,
            &self.home,
            &self.signins,
            now,
            &Local,
        );
        self.snapshot_id += 1;
        data.snapshot_id = self.snapshot_id;
        data.file_count = self.rollout_count();
        data.pricing_error = self.pricing_error.clone();
        // A successfully read copy is always chosen for its rollout, so a
        // failure of another copy costs nothing.
        let read: HashSet<&str> = self
            .files
            .values()
            .filter(|state| state.readable())
            .map(|state| state.key.as_str())
            .collect();
        let superseded = |path: &Path| {
            self.files
                .get(path)
                .is_some_and(|state| read.contains(state.key.as_str()))
        };
        data.read_errors = self
            .home
            .problem()
            .into_iter()
            .chain(
                self.read_errors
                    .iter()
                    .filter(|(path, _)| !superseded(path))
                    .map(|(path, message)| ReadError {
                        path: path.display().to_string(),
                        message: message.clone(),
                    }),
            )
            .collect();
        data
    }

    /// Every day and all time by model and effort, from what was read so far.
    pub fn history(&mut self, now: DateTime<Utc>) -> CodexHistory {
        report::history(&self.ledger(), now, &Local)
    }
}

/// Reads a plain rollout on from its offset, or again from the start when
/// what was read before no longer matches. Only complete lines are taken: a
/// last line without its newline counts once it is valid JSON, otherwise it
/// is still being written and is read next time.
fn read_plain(path: &Path, state: &mut FileState) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let stamp = FileStamp::from(&file.metadata()?);
    let replaced = state
        .stamp
        .is_some_and(|old| old.created != stamp.created && old.created.is_some());
    let reread = replaced
        || stamp.len < state.offset
        || (state.offset > 0 && Some(anchors(&mut file, state.offset)?) != state.anchors);
    if reread {
        // A fresh read replaces the old one only once it succeeded.
        let mut fresh = FileState::new(state.key.clone(), state.root, state.encoding);
        read_lines(&mut file, &mut fresh)?;
        fresh.anchors = Some(anchors(&mut file, fresh.offset)?);
        fresh.stamp = Some(stamp);
        *state = fresh;
        return Ok(true);
    }
    state.stamp = Some(stamp);
    // Lines read before a failure stay counted.
    let before = state.log.signature();
    let result = read_lines(&mut file, state);
    let changed = state.log.signature() != before;
    result?;
    state.anchors = Some(anchors(&mut file, state.offset)?);
    Ok(changed)
}

fn read_lines(file: &mut File, state: &mut FileState) -> io::Result<()> {
    file.seek(SeekFrom::Start(state.offset))?;
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
        if !line.trim_ascii().is_empty() {
            state.log.ingest(line.trim_ascii_end());
        }
        state.offset += read as u64;
    }
    Ok(())
}

/// A rollout to read, taken out of the store while a reader thread has it.
struct Job {
    path: PathBuf,
    state: FileState,
    /// Its metadata as the walk found it.
    stamp: FileStamp,
}

/// Reads every job, on up to `READERS` threads taking the largest files first.
fn read_all(mut jobs: Vec<Job>) -> Vec<(Job, io::Result<bool>)> {
    let workers = thread::available_parallelism()
        .map_or(1, usize::from)
        .min(READERS)
        .min(jobs.len());
    if workers <= 1 {
        return jobs.into_iter().map(read_job).collect();
    }
    jobs.sort_by_key(|job| job.stamp.len);
    let queue = Mutex::new(jobs);
    thread::scope(|scope| {
        let readers: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let job = queue.lock().unwrap_or_else(PoisonError::into_inner).pop();
                        let Some(job) = job else {
                            break done;
                        };
                        done.push(read_job(job));
                    }
                })
            })
            .collect();
        readers
            .into_iter()
            .flat_map(|reader| {
                reader
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

fn read_job(mut job: Job) -> (Job, io::Result<bool>) {
    let result = match job.state.encoding {
        Encoding::Plain => read_plain(&job.path, &mut job.state),
        Encoding::Zstd => read_compressed(&job.path, &mut job.state),
    };
    (job, result)
}

/// Reads a compressed rollout whole, decoding as it goes. It replaces what
/// was read before only once read through.
fn read_compressed(path: &Path, state: &mut FileState) -> io::Result<bool> {
    let file = File::open(path)?;
    let stamp = FileStamp::from(&file.metadata()?);
    let mut reader = BufReader::new(ZstReader::new(BufReader::new(file)));
    let mut log = FileLog::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if !line.trim_ascii().is_empty() {
            log.ingest(line.trim_ascii_end());
        }
    }
    state.log = log;
    state.stamp = Some(stamp);
    Ok(true)
}

/// Checksums of the first bytes and of the bytes just before `offset`.
fn anchors(file: &mut File, offset: u64) -> io::Result<(u32, u32)> {
    let len = offset.min(ANCHOR_BYTES) as usize;
    let mut buffer = vec![0; len];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut buffer)?;
    let head = crc32fast::hash(&buffer);
    file.seek(SeekFrom::Start(offset - len as u64))?;
    file.read_exact(&mut buffer)?;
    Ok((head, crc32fast::hash(&buffer)))
}

/// `session_index.jsonl`: one `{id, thread_name}` per line, a later line
/// renaming the thread.
fn parse_titles(bytes: &[u8]) -> Titles {
    #[derive(Deserialize)]
    struct Line {
        id: Option<String>,
        thread_name: Option<String>,
    }
    let mut titles = Titles::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let Ok(Line {
            id: Some(id),
            thread_name: Some(name),
        }) = serde_json::from_slice::<Line>(line)
        else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() {
            titles.insert(id, name.to_owned());
        }
    }
    titles
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cc-deskmeter-codex-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("sessions/2026/10/19")).unwrap();
            fs::create_dir_all(dir.join("archived_sessions")).unwrap();
            Self(dir)
        }

        fn store(&self) -> CodexStore {
            CodexStore::new(CodexHome::from(self.0.clone()))
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const META: &str = r#"{"timestamp":"2026-10-19T07:49:21.133Z","type":"session_meta","payload":{"id":"t1","timestamp":"2026-10-19T07:49:15.910Z","cwd":"/work"}}"#;
    const TURN: &str = r#"{"timestamp":"2026-10-19T07:49:22.000Z","type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-6-astra","effort":"high"}}"#;

    fn count(total: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-10-19T07:50:{:02}.000Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total},"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}},"last_token_usage":{{"input_tokens":1,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}}}}}}}"#,
            total % 60
        )
    }

    fn total(store: &mut CodexStore) -> u64 {
        store.snapshot(Utc::now()).total.usage.tokens.total()
    }

    fn append(path: &Path, text: &str) {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn reads_appends_and_waits_for_partial_lines() {
        let home = TempHome::new("append");
        let path = home
            .0
            .join("sessions/2026/10/19/rollout-2026-10-19T03-49-15-t1.jsonl");
        append(&path, &format!("{META}\n{}\n", count(100)));
        let mut store = home.store();
        assert!(store.refresh());
        assert_eq!(total(&mut store), 100);
        assert!(!store.refresh(), "nothing new");

        let line = count(250);
        append(&path, &line[..line.len() / 2]);
        store.refresh();
        assert_eq!(total(&mut store), 100, "half a line is still being written");
        append(&path, &format!("{}\n", &line[line.len() / 2..]));
        assert!(store.refresh());
        assert_eq!(total(&mut store), 250);
    }

    #[test]
    fn a_rewritten_rollout_is_read_again() {
        let home = TempHome::new("rewrite");
        let path = home
            .0
            .join("sessions/2026/10/19/rollout-2026-10-19T03-49-15-t1.jsonl");
        fs::write(&path, format!("{META}\n{}\n", count(100))).unwrap();
        let mut store = home.store();
        store.refresh();
        // Same length, different content (a migration rewriting in place).
        fs::write(&path, format!("{META}\n{}\n", count(300))).unwrap();
        assert!(store.refresh_paths(&HashSet::from([path.clone()])));
        assert_eq!(total(&mut store), 300);
        // Shorter: read from the start.
        fs::write(&path, format!("{META}\n{}\n", count(7))).unwrap();
        store.refresh();
        assert_eq!(total(&mut store), 7);
    }

    #[test]
    fn each_rollout_counts_once_whatever_its_copies() {
        let home = TempHome::new("copies");
        let name = "rollout-2026-10-19T03-49-15-t1.jsonl";
        let content = format!("{META}\n{}\n", count(100));
        let live = home.0.join("sessions/2026/10/19").join(name);
        fs::write(&live, &content).unwrap();
        // Codex compresses a rollout before deleting the plain copy.
        let compressed = home
            .0
            .join("sessions/2026/10/19")
            .join(format!("{name}.zst"));
        fs::write(&compressed, zstd_frame(content.as_bytes())).unwrap();
        fs::write(home.0.join("archived_sessions").join(name), &content).unwrap();
        let mut store = home.store();
        store.refresh();
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 100);
        assert_eq!(data.file_count, 1);
        assert!(data.read_errors.is_empty(), "{:?}", data.read_errors);

        fs::remove_file(&live).unwrap();
        fs::remove_file(home.0.join("archived_sessions").join(name)).unwrap();
        assert!(store.refresh());
        assert_eq!(total(&mut store), 100, "the compressed copy remains");
        fs::remove_file(&compressed).unwrap();
        assert!(store.refresh());
        assert_eq!(total(&mut store), 0);
    }

    #[test]
    fn a_compressed_copy_still_being_written_is_not_an_error() {
        let home = TempHome::new("half-written");
        let name = "rollout-2026-10-19T03-49-15-t1.jsonl";
        let content = format!(
            "{META}
{}
",
            count(100)
        );
        let dir = home.0.join("sessions/2026/10/19");
        fs::write(dir.join(name), &content).unwrap();
        let mut frame = zstd_frame(content.as_bytes());
        frame.truncate(frame.len() / 2);
        fs::write(dir.join(format!("{name}.zst")), frame).unwrap();
        let mut store = home.store();
        store.refresh();
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 100);
        assert!(data.read_errors.is_empty(), "{:?}", data.read_errors);
    }

    #[test]
    fn a_successful_compressed_copy_supersedes_a_failed_plain_read() {
        let home = TempHome::new("failed-plain");
        let name = "rollout-2026-10-19T03-49-15-t1.jsonl";
        let dir = home.0.join("sessions/2026/10/19");
        let compressed = dir.join(format!("{name}.zst"));
        fs::write(
            &compressed,
            zstd_frame(format!("{META}\n{}\n", count(100)).as_bytes()),
        )
        .unwrap();
        let mut store = home.store();
        store.refresh();
        let plain = dir.join(name);
        let mut failed = FileState::new(name.into(), 0, Encoding::Plain);
        failed.failed = true;
        store.files.insert(plain.clone(), failed);
        store.read_errors.insert(plain, "read failed".into());

        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 100);
        assert_eq!(data.file_count, 1);
        assert!(data.read_errors.is_empty(), "{:?}", data.read_errors);

        store.files.get_mut(&compressed).unwrap().failed = true;
        store.read_errors.insert(compressed, "read failed".into());
        store.generation += 1;
        assert_eq!(store.snapshot(Utc::now()).read_errors.len(), 2);
    }

    #[test]
    fn a_damaged_compressed_rollout_is_reported_not_counted() {
        let home = TempHome::new("damaged");
        let path = home
            .0
            .join("sessions/2026/10/19/rollout-2026-10-19T03-49-15-t1.jsonl.zst");
        let mut frame = zstd_frame(format!("{META}\n{}\n", count(100)).as_bytes());
        frame.truncate(frame.len() - 3);
        fs::write(&path, frame).unwrap();
        let mut store = home.store();
        store.refresh();
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 0);
        assert_eq!(data.read_errors.len(), 1);
    }

    #[test]
    fn a_damaged_compressed_rollout_is_not_decoded_again_until_it_changes() {
        let home = TempHome::new("damaged-again");
        let path = home
            .0
            .join("sessions/2026/10/19/rollout-2026-10-19T03-49-15-t1.jsonl.zst");
        let frame = zstd_frame(format!("{META}\n{}\n", count(100)).as_bytes());
        // The rollout's frame, then a skippable frame of `announced` bytes with
        // three there: cut short at 16, whole at 3.
        let with_skippable = |announced: u8| {
            let mut bytes = frame.clone();
            bytes.extend([0x50, 0x2a, 0x4d, 0x18, announced, 0, 0, 0, 1, 2, 3]);
            bytes
        };
        fs::write(&path, with_skippable(16)).unwrap();
        let mut store = home.store();
        store.refresh();
        assert_eq!(store.snapshot(Utc::now()).read_errors.len(), 1);

        // Repaired in place, as long as before and its modification time kept:
        // to the walk nothing changed, so it is not decoded again.
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(&with_skippable(3)).unwrap();
        file.set_modified(modified).unwrap();
        drop(file);
        assert!(!store.refresh());
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 0);
        assert_eq!(data.read_errors.len(), 1, "the error is still shown");

        // A notification reads it all the same, as a rescan does.
        assert!(store.refresh_paths(&HashSet::from([path.clone()])));
        let data = store.snapshot(Utc::now());
        assert_eq!(data.total.usage.tokens.total(), 100);
        assert!(data.read_errors.is_empty(), "{:?}", data.read_errors);
    }

    #[test]
    fn local_prices_apply_on_refresh_and_bad_files_keep_the_last_valid_ones() {
        let home = TempHome::new("pricing");
        let path = home
            .0
            .join("sessions/2026/10/19/rollout-2026-10-19T03-49-15-t1.jsonl");
        // 100K uncached Astra input: $1 at the built-in $10 per million.
        fs::write(&path, format!("{META}\n{TURN}\n{}\n", count(100_000))).unwrap();
        let prices = home.0.join("pricing.json");
        let mut store = home.store().with_pricing_file(Some(prices.clone()));
        let cost = |store: &mut CodexStore| store.snapshot(Utc::now()).total.usage.cost_usd;

        store.refresh();
        assert!(
            (cost(&mut store) - 1.0).abs() < 1e-9,
            "no file: built-in price"
        );
        assert_eq!(store.snapshot(Utc::now()).custom_prices, 0);

        // The same file reprices Claude's models; their entries leave Codex alone.
        fs::write(
            &prices,
            r#"{ "claude-opus-5-5": { "input": 9, "output": 45 }, "gpt-6-astra": { "input": 20, "output": 100 } }"#,
        )
        .unwrap();
        assert!(
            store.refresh(),
            "an edited price file reprices without new usage"
        );
        let data = store.snapshot(Utc::now());
        assert!((data.total.usage.cost_usd - 2.0).abs() < 1e-9);
        assert_eq!((data.custom_prices, data.pricing_error), (1, None));
        assert!(!store.refresh(), "unchanged file is not reparsed");

        fs::write(&prices, r#"{ "gpt-6-astra": { "input": 20 "#).unwrap();
        let generation = store.generation();
        assert!(!store.refresh(), "prices are unchanged");
        assert!(
            store.generation() > generation,
            "the error is still published"
        );
        let data = store.snapshot(Utc::now());
        assert!(
            (data.total.usage.cost_usd - 2.0).abs() < 1e-9,
            "last valid prices stay"
        );
        assert!(data.pricing_error.is_some());

        fs::remove_file(&prices).unwrap();
        assert!(store.refresh());
        let data = store.snapshot(Utc::now());
        assert!((data.total.usage.cost_usd - 1.0).abs() < 1e-9);
        assert_eq!((data.custom_prices, data.pricing_error), (0, None));
    }

    #[test]
    fn titles_come_from_the_session_index() {
        assert_eq!(
            parse_titles(
                br#"{"id":"a","thread_name":"First","updated_at":"x"}
{"id":"b","thread_name":"  "}
not json
{"id":"a","thread_name":"Renamed"}"#
            ),
            Titles::from([("a".into(), "Renamed".into())])
        );
    }

    /// `cargo test --release -- --ignored --nocapture scan_real_codex_logs`
    /// reads the discovered Codex home (or `CODEX_HOME`). Set `CODEX_NOW`
    /// (RFC 3339) to fix the clock and `CODEX_DUMP` to a folder to write the
    /// snapshot and per-thread totals there.
    #[test]
    #[ignore = "reads the real Codex rollouts"]
    fn scan_real_codex_logs() {
        let mut store = CodexStore::new(CodexHome::discover());
        let started = std::time::Instant::now();
        store.refresh();
        let scan = started.elapsed();
        let now = std::env::var("CODEX_NOW")
            .ok()
            .and_then(|now| DateTime::parse_from_rfc3339(&now).ok())
            .map_or_else(Utc::now, |now| now.with_timezone(&Utc));
        let started = std::time::Instant::now();
        let data = store.snapshot(now);
        let first = started.elapsed();
        let started = std::time::Instant::now();
        store.snapshot(now);
        let tick = started.elapsed();
        let ledger = store.ledger();
        if let Some(dir) = std::env::var_os("CODEX_DUMP") {
            let dir = PathBuf::from(dir);
            fs::write(
                dir.join("rust-snapshot.json"),
                serde_json::to_string_pretty(&data).unwrap(),
            )
            .unwrap();
            let threads: Vec<_> = ledger
                .threads
                .iter()
                .map(|thread| {
                    let own = thread.usage.tokens;
                    serde_json::json!({
                        "id": &*thread.id,
                        "own": [own.input, own.cached, own.output, own.reasoning],
                        "costUsd": thread.usage.cost_usd,
                        "turns": thread.turns,
                        "compactions": thread.compactions,
                        "peak": thread.peak,
                        "peakWindow": thread.peak_window,
                        "model": thread.model.as_deref(),
                        "effort": thread.effort.as_deref(),
                        "last": thread.last_activity,
                    })
                })
                .collect();
            fs::write(
                dir.join("rust-threads.json"),
                serde_json::to_string_pretty(&threads).unwrap(),
            )
            .unwrap();
        }
        println!(
            "rollouts: {}, threads: {}, responses: {}, turns: {}, readings: {}, total: {:?}",
            data.file_count,
            ledger.threads.len(),
            ledger.responses.len(),
            ledger.turns.len(),
            ledger.readings.len(),
            ledger.total
        );
        println!("read errors: {:?}", data.read_errors);
        println!("initial scan: {scan:?}; first snapshot: {first:?}; next: {tick:?}");

        let started = std::time::Instant::now();
        let history = store.history(now);
        let took = started.elapsed();
        let days: u64 = history
            .daily
            .iter()
            .map(|day| day.usage.tokens.total())
            .sum();
        let top = history
            .models
            .first()
            .map(|row| (row.model.as_deref(), row.usage.tokens.total()));
        println!(
            "history: {} days from {}, {days} of {} tokens on them, top model {top:?}, {} efforts, in {took:?}",
            history.daily.len(),
            history.daily[0].date,
            history.total.usage.tokens.total(),
            history.efforts.len()
        );
        // Only responses logged ahead of the clock fall on no day.
        assert!(days <= history.total.usage.tokens.total());
    }

    fn zstd_frame(data: &[u8]) -> Vec<u8> {
        ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
    }
}
