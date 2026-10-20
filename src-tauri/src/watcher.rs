//! Watches every transcript root and pushes fresh usage data to the
//! frontend as the `token-updated` event, and to the tray icon.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

use crate::tray;
use crate::usage::{self, UsageData, UsageStore};

pub const USAGE_EVENT: &str = "token-updated";

/// Claude Code appends several lines per response; coalesce them into one update.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// Re-emit periodically so the block countdown, burn rate and "today" stay
/// current without file activity. Also retries watchers that failed to start
/// or whose root does not exist yet.
const TICK: Duration = Duration::from_secs(30);
/// Full integrity checks are independent of notifications and never hash a
/// transcript while holding the store lock. Each pass audits the transcripts
/// that grew since their last check.
const VALIDATION_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Every this many passes (hourly), unchanged transcripts are audited too.
const FULL_VALIDATION_EVERY: u64 = 12;

type NotifyResult = notify::Result<Event>;

pub fn spawn(app: AppHandle, store: Arc<Mutex<UsageStore>>) {
    let validator_app = app.clone();
    let validator_store = Arc::clone(&store);
    if let Err(err) = std::thread::Builder::new()
        .name("usage-validator".into())
        .spawn(move || {
            for pass in 1u64.. {
                std::thread::sleep(VALIDATION_INTERVAL);
                let include_unchanged = pass % FULL_VALIDATION_EVERY == 0;
                let jobs = usage::lock(&validator_store).validation_jobs(include_unchanged);
                for job in jobs {
                    let result = job.check();
                    let data = {
                        let mut store = usage::lock(&validator_store);
                        store
                            .apply_validation(result)
                            .then(|| store.snapshot(Utc::now()))
                    };
                    if let Some(data) = data {
                        emit(&validator_app, &data);
                    }
                }
            }
        })
    {
        eprintln!("cc-deskmeter: failed to start usage validator: {err}");
    }
    let spawned = std::thread::Builder::new()
        .name("usage-watcher".into())
        .spawn(move || run(&app, &store));
    if let Err(err) = spawned {
        eprintln!("cc-deskmeter: failed to start usage watcher: {err}");
    }
}

fn run(app: &AppHandle, store: &Mutex<UsageStore>) {
    let roots = usage::lock(store).roots().to_vec();
    let (tx, rx) = mpsc::channel::<NotifyResult>();
    // One watcher per root, so each comes and goes with its own folder.
    let mut watchers: Vec<Option<RecommendedWatcher>> = roots.iter().map(|_| None).collect();
    let mut last_generation = None;

    publish(app, store, &mut last_generation, true, &HashSet::new());
    let mut next_tick = Instant::now() + TICK;
    loop {
        for (root, watcher) in roots.iter().zip(&mut watchers) {
            if watcher.is_none() {
                *watcher = start_watcher(root, tx.clone());
            }
        }
        let Ok(update) = wait_for_update(&rx, &mut next_tick, TICK, DEBOUNCE) else {
            return;
        };
        if update.relevant || update.force {
            publish(
                app,
                store,
                &mut last_generation,
                update.force,
                &update.paths,
            );
        }
        for (root, watcher) in roots.iter().zip(&mut watchers) {
            if !root.is_dir() {
                // The watched directory went away; re-create the watcher once it is back.
                *watcher = None;
            }
        }
    }
}

#[derive(Default)]
struct Update {
    relevant: bool,
    force: bool,
    paths: HashSet<PathBuf>,
}

impl Update {
    fn observe(&mut self, event: NotifyResult) {
        if is_relevant(&event) {
            self.relevant = true;
            if let Ok(event) = event {
                self.paths.extend(event.paths);
            }
        }
    }
}

/// The periodic deadline belongs to the scheduler, not to the next receive.
/// Neither incoming events nor a debounce window may postpone it.
fn wait_for_update(
    rx: &Receiver<NotifyResult>,
    next_tick: &mut Instant,
    tick: Duration,
    debounce: Duration,
) -> Result<Update, RecvTimeoutError> {
    let mut update = Update::default();
    match rx.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
        Ok(first) => {
            update.observe(first);
            let deadline = (Instant::now() + debounce).min(*next_tick);
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(next) => update.observe(next),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(err) => return Err(err),
                }
            }
        }
        Err(RecvTimeoutError::Timeout) => {}
        Err(err) => return Err(err),
    }
    let now = Instant::now();
    if now >= *next_tick {
        update.force = true;
        *next_tick = now + tick;
    }
    Ok(update)
}

fn start_watcher(root: &Path, tx: mpsc::Sender<NotifyResult>) -> Option<RecommendedWatcher> {
    if !root.is_dir() {
        return None;
    }
    let result = notify::recommended_watcher(tx).and_then(|mut watcher| {
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map(|()| watcher)
    });
    match result {
        Ok(watcher) => Some(watcher),
        Err(err) => {
            eprintln!("cc-deskmeter: cannot watch {}: {err}", root.display());
            None
        }
    }
}

/// Reads are noise; anything else may mean new lines, files or folders. The
/// refresh itself is cheap and decides whether usage actually changed.
fn is_relevant(event: &NotifyResult) -> bool {
    match event {
        Ok(event) => !matches!(event.kind, EventKind::Access(_)),
        Err(err) => {
            eprintln!("cc-deskmeter: watch error: {err}");
            true
        }
    }
}

/// Refreshes the store and emits the snapshot if usage changed since the last
/// emit, or unconditionally when `force` is set.
fn publish(
    app: &AppHandle,
    store: &Mutex<UsageStore>,
    last_generation: &mut Option<u64>,
    force: bool,
    paths: &HashSet<PathBuf>,
) {
    let data = {
        let mut store = usage::lock(store);
        if !paths.is_empty() {
            store.refresh_paths(paths);
        }
        if force || paths.is_empty() {
            store.refresh();
        }
        let generation = store.generation();
        if !force && *last_generation == Some(generation) {
            return;
        }
        *last_generation = Some(generation);
        store.snapshot(Utc::now())
    };
    emit(app, &data);
}

fn emit(app: &AppHandle, data: &UsageData) {
    tray::push(app, data);
    if let Err(err) = app.emit(USAGE_EVENT, data) {
        eprintln!("cc-deskmeter: failed to emit {USAGE_EVENT}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, ModifyKind};

    #[test]
    fn a_due_tick_is_published_even_with_queued_irrelevant_events() {
        let (tx, rx) = mpsc::channel();
        for _ in 0..100 {
            tx.send(Ok(Event::new(EventKind::Access(AccessKind::Any))))
                .unwrap();
        }
        let mut deadline = Instant::now();
        let update = wait_for_update(&rx, &mut deadline, TICK, DEBOUNCE).unwrap();
        assert!(update.force);
        assert!(!update.relevant);
        assert!(deadline > Instant::now());
    }

    #[test]
    fn repeated_events_do_not_postpone_tick_and_debounce_is_capped() {
        let (tx, rx) = mpsc::channel();
        let tick = Duration::from_millis(40);
        let mut deadline = Instant::now() + tick;
        let first_deadline = deadline;
        for _ in 0..10_000 {
            tx.send(Ok(Event::new(EventKind::Modify(ModifyKind::Any))
                .add_path(PathBuf::from("transcript.jsonl"))))
                .unwrap();
        }
        let mut forced = false;
        while Instant::now() < first_deadline + Duration::from_secs(1) {
            let update =
                wait_for_update(&rx, &mut deadline, tick, Duration::from_millis(5)).unwrap();
            if update.force {
                forced = true;
                break;
            }
            assert_eq!(deadline, first_deadline);
            assert!(update.paths.contains(Path::new("transcript.jsonl")));
        }
        assert!(forced, "file activity must not starve the timer");

        tx.send(Ok(Event::new(EventKind::Modify(ModifyKind::Any))))
            .unwrap();
        deadline = Instant::now() + Duration::from_millis(20);
        let update = wait_for_update(&rx, &mut deadline, tick, Duration::from_secs(10)).unwrap();
        assert!(update.force, "debouncing must end at the periodic deadline");
    }
}
