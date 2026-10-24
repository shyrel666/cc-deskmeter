//! The accounting: every response's tokens, counted once, in the thread
//! that spent them.
//!
//! Each rule was checked on 149 real threads against Codex's own per-thread
//! totals (`threads.tokens_used` in `state_5.sqlite`): every thread matched
//! to the token, once the replayed history and the compaction requests that
//! total leaves out (below) were accounted for.
//!
//! - **Responses.** Since Codex 0.153 every response has a `token_usage_record`
//!   (counted once per `response_id`, across files): the latest records that
//!   add up to a counter advance (cache writes aside, which a record may log
//!   and the counter not) are the responses it covers. Before, and for any
//!   counter advance no record covers, a response is the advance of the
//!   thread's cumulative `token_count` total. Repeated snapshots (210 in the
//!   logs, among them the one after each compaction whose `last_token_usage`
//!   holds the new context size, not usage) add nothing.
//! - **Compactions.** Remote compaction requests are logged as records the
//!   cumulative total never includes; they count. Before 0.153 a compaction
//!   logged no usage at all, so it is only counted as a compaction.
//! - **Pages.** A thread can span several rollouts (`…_<window>.jsonl` after
//!   a compaction); its counter carries from one page to the next.
//! - **Replays.** A forked or spawned thread first replays its parent's
//!   history. Since 0.162 `subagent_history_start_ordinal` marks where its own
//!   records begin (older rollouts carry a value past their end, written by a
//!   later migration, which says nothing). Otherwise its leading cumulative
//!   snapshots that the parent logged are the parent's; and with the parent's
//!   rollout gone, everything before its first `trigger_turn` marker.
//! - **First snapshot.** A thread's very first snapshot counts from zero, so a
//!   turn whose own `token_count` was never logged (an aborted one) still
//!   counts. A page that continues another (`history_base`) whose rollout is
//!   gone counts only its own advance.
//! - **Turns.** A turn counts once it has a counted response, so a parent's
//!   turns replayed in a subagent's rollout never do.
//! - **Readings.** Each `token_count` also carries the signed-in account's
//!   limits; a response keeps the reading logged with it (see `report` for
//!   whose account that is). The limits leave that response out (see
//!   `accounts`), so a reading notes when it was logged.
//! - **Cost.** Each response is priced on its own, at its turn's model (see
//!   `pricing`), so a thread's or a day's cost is the sum of its responses'.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::parser::{Entry, FileLog, Limits, Record, SessionMeta};
use super::pricing::Prices;
use super::types::{CodexCostSplit, CodexTokens, CodexUsage};
use crate::usage::Pricing;

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: Arc<str>,
    /// Index of the thread this one was forked or spawned from, if its rollout exists.
    pub parent: Option<usize>,
    /// The topmost ancestor reached through subagent links (itself for a fork).
    pub root: usize,
    pub subagent: bool,
    pub account: Option<Arc<str>>,
    pub cwd: Option<String>,
    pub nickname: Option<String>,
    pub first_prompt: Option<String>,
    /// Its own usage, subagents not included.
    pub usage: CodexUsage,
    pub turns: u64,
    pub compactions: u64,
    /// Largest context (input + output) of one of its responses, and the
    /// model's window at that response.
    pub peak: u64,
    pub peak_window: Option<u64>,
    /// Model and effort of its latest response.
    pub model: Option<Arc<str>>,
    pub effort: Option<Arc<str>>,
    pub last_activity: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub at: DateTime<Utc>,
    pub thread: usize,
    pub tokens: CodexTokens,
    /// API-equivalent cost, USD, by kind of token; `None` when the model has
    /// no price.
    pub cost: Option<CodexCostSplit>,
    pub model: Option<Arc<str>>,
    pub effort: Option<Arc<str>>,
    /// The model's context window when the response was logged.
    pub window: Option<u64>,
    /// Index into `Ledger::readings` of the limits logged with the response
    /// (with its `token_count`; a usage record logged without one takes the
    /// thread's latest). `None` for providers that report no limits.
    pub reading: Option<usize>,
}

impl Response {
    pub fn usage(&self) -> CodexUsage {
        CodexUsage::of(self.tokens, self.cost.map(|cost| cost.total()))
    }
}

#[derive(Debug, Clone)]
pub struct Turn {
    /// When its first counted response was logged.
    pub at: DateTime<Utc>,
    pub model: Option<Arc<str>>,
    pub effort: Option<Arc<str>>,
}

#[derive(Debug, Clone)]
pub struct Compaction {
    pub at: DateTime<Utc>,
}

/// The limits of the account signed in when a thread logged them: not
/// necessarily the account that started the thread.
#[derive(Debug, Clone)]
pub struct Reading {
    pub at: DateTime<Utc>,
    /// When the response logged with the limits was, else `at`: the limits
    /// show use before that response, none of it or of anything logged since.
    pub since: DateTime<Utc>,
    /// No earlier than this the request these limits came with was sent: its
    /// turn's start or the thread's previous response, whichever was later.
    /// `None` when the thread logged neither.
    pub asked: Option<DateTime<Utc>>,
    pub thread: usize,
    pub limits: Arc<Limits>,
    /// The first reading of a new top-level thread: logged under the account
    /// that started it, so it tells whose limits these are.
    pub birth: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Ledger {
    /// In the order they were started.
    pub threads: Vec<Thread>,
    pub responses: Vec<Response>,
    pub turns: Vec<Turn>,
    pub compactions: Vec<Compaction>,
    pub readings: Vec<Reading>,
    pub total: CodexUsage,
    /// Entries of `pricing.json` that priced a response.
    pub custom_prices: usize,
}

/// A rollout file, named as `rollout-….jsonl` whatever its encoding.
type Page<'a> = (&'a str, &'a FileLog, &'a SessionMeta);

impl Ledger {
    /// Accounts for every rollout given, one file per rollout, and prices
    /// each response with `pricing`'s overrides and the built-in table.
    pub fn build<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a FileLog)>,
        pricing: &Pricing,
    ) -> Self {
        let mut pages: HashMap<Arc<str>, Vec<Page<'a>>> = HashMap::new();
        for (name, log) in files {
            if let Some(meta) = &log.meta {
                pages
                    .entry(Arc::clone(&meta.id))
                    .or_default()
                    .push((name, log, meta));
            }
        }
        let started = |page: &Page<'a>| (page.2.started, page.0);
        for list in pages.values_mut() {
            list.sort_by(|a, b| started(a).cmp(&started(b)));
        }
        let mut order: Vec<Arc<str>> = pages.keys().cloned().collect();
        order.sort_by(|a, b| started(&pages[a][0]).cmp(&started(&pages[b][0])));
        let index: HashMap<&str, usize> =
            order.iter().enumerate().map(|(i, id)| (&**id, i)).collect();

        // Every cumulative snapshot each thread logged, to tell the ones a
        // child replays from its own.
        let snapshots: HashMap<&str, HashSet<CodexTokens>> = pages
            .iter()
            .map(|(id, list)| {
                let totals = list
                    .iter()
                    .flat_map(|(_, log, _)| &log.entries)
                    .filter_map(|entry| match entry.record {
                        Record::TokenCount {
                            total: Some(total), ..
                        } => Some(total),
                        _ => None,
                    })
                    .collect();
                (&**id, totals)
            })
            .collect();

        let mut threads: Vec<Thread> = order
            .iter()
            .map(|id| {
                let (_, log, meta) = pages[id][0];
                Thread {
                    id: Arc::clone(id),
                    parent: meta
                        .parent
                        .as_deref()
                        .and_then(|parent| index.get(parent).copied()),
                    root: 0,
                    subagent: meta.subagent,
                    account: meta.account.clone(),
                    cwd: meta.cwd.clone(),
                    nickname: meta.nickname.clone(),
                    first_prompt: log.first_prompt.clone(),
                    usage: CodexUsage::default(),
                    turns: 0,
                    compactions: 0,
                    peak: 0,
                    peak_window: None,
                    model: None,
                    effort: None,
                    last_activity: None,
                }
            })
            .collect();
        for i in 0..threads.len() {
            let mut root = i;
            // Bounded, should the logs ever link threads in a cycle.
            for _ in 0..threads.len() {
                match threads[root].parent {
                    Some(parent) if threads[root].subagent && parent != i => root = parent,
                    _ => break,
                }
            }
            threads[i].root = root;
        }

        let mut ledger = Self::default();
        let mut seen = HashSet::new();
        let mut prices = Prices::new(pricing);
        for (i, id) in order.iter().enumerate() {
            let list = &pages[id];
            let parent = list[0].2.parent.as_deref();
            let parent_snapshots = parent
                .and_then(|parent| snapshots.get(parent))
                .filter(|snapshots| !snapshots.is_empty());
            let mut counter = ThreadCounter {
                index: i,
                thread: &mut threads[i],
                ledger: &mut ledger,
                seen: &mut seen,
                prices: &mut prices,
                state: State::default(),
            };
            // Only a thread started here, not one continued or spawned, made
            // its first request under the account that started it.
            counter.state.born = list[0].2.continues || parent.is_some();
            for (page_index, (_, log, meta)) in list.iter().enumerate() {
                let child = page_index == 0 && parent.is_some();
                let fresh = page_index == 0 && !meta.continues;
                counter.page(log, meta, child, fresh, parent_snapshots);
            }
        }
        ledger.threads = threads;
        ledger.custom_prices = prices.custom_count();
        ledger
    }
}

/// A turn's model and effort.
type Settings = (Option<Arc<str>>, Option<Arc<str>>);

/// What the latest records of a thread said, carried across its pages.
#[derive(Default)]
struct State {
    /// The thread's cumulative total at the last snapshot.
    prev: Option<CodexTokens>,
    /// Model and effort per turn, from its `turn_context`.
    contexts: HashMap<Option<Arc<str>>, Settings>,
    model: Option<Arc<str>>,
    effort: Option<Arc<str>>,
    turn: Option<Arc<str>>,
    window: Option<u64>,
    /// Turns that already have a counted response.
    turns: HashSet<Option<Arc<str>>>,
    /// The thread's latest reading.
    reading: Option<usize>,
    /// The thread's first reading was taken (or cannot tell an account).
    born: bool,
    /// When the thread's latest turn started or its latest response was
    /// logged: its next request goes out after that (see `Reading::asked`).
    asked: Option<DateTime<Utc>>,
}

impl State {
    /// The thread's next request goes out after `at`.
    fn ask_after(&mut self, at: DateTime<Utc>) {
        self.asked = Some(self.asked.map_or(at, |asked| asked.max(at)));
    }
}

struct ThreadCounter<'l, 'p> {
    index: usize,
    thread: &'l mut Thread,
    ledger: &'l mut Ledger,
    seen: &'l mut HashSet<Arc<str>>,
    prices: &'l mut Prices<'p>,
    state: State,
}

/// Where a page's records stand relative to the history it replays.
struct Replay<'a> {
    /// Still inside the replayed history.
    active: bool,
    /// `subagent_history_start_ordinal`, when the page has records from it on.
    start: Option<u64>,
    parent_snapshots: Option<&'a HashSet<CodexTokens>>,
    /// A `trigger_turn` marker was seen, or the page has none.
    triggered: bool,
}

impl ThreadCounter<'_, '_> {
    fn page(
        &mut self,
        log: &FileLog,
        meta: &SessionMeta,
        child: bool,
        fresh: bool,
        parent_snapshots: Option<&HashSet<CodexTokens>>,
    ) {
        let start = meta
            .history_start
            .filter(|start| log.max_ordinal.is_some_and(|max| max >= *start));
        let mut replay = Replay {
            active: child,
            start,
            parent_snapshots: if child { parent_snapshots } else { None },
            triggered: !log
                .entries
                .iter()
                .any(|entry| entry.record == Record::AgentTrigger),
        };
        let mut pending: Vec<&Entry> = Vec::new();
        for entry in &log.entries {
            let replayed = replay.active && replay.start.is_some_and(|start| entry.ordinal < start);
            if replay.active && replay.start.is_some_and(|start| entry.ordinal >= start) {
                replay.active = false;
            }
            match &entry.record {
                Record::AgentTrigger => replay.triggered = true,
                Record::TurnContext {
                    turn,
                    model,
                    effort,
                } => {
                    let state = &mut self.state;
                    state
                        .contexts
                        .insert(turn.clone(), (model.clone(), effort.clone()));
                    state.model = model.clone();
                    state.effort = effort.clone();
                    state.turn = turn.clone();
                }
                Record::TurnStarted {
                    turn,
                    context_window,
                } => {
                    self.state.turn = turn.clone();
                    if context_window.is_some() {
                        self.state.window = *context_window;
                    }
                    self.state.ask_after(entry.at);
                }
                Record::Compacted { .. } => {
                    if !(replay.active || replayed) {
                        self.thread.compactions += 1;
                        self.touch(entry.at);
                        self.ledger.compactions.push(Compaction { at: entry.at });
                    }
                }
                Record::Usage { .. } => {
                    if !replayed {
                        pending.push(entry);
                    }
                }
                Record::TokenCount {
                    total,
                    last,
                    context_window,
                    limits,
                } => {
                    let counted = self.ledger.responses.len();
                    if let Some(total) = total {
                        if context_window.is_some() {
                            self.state.window = *context_window;
                        }
                        self.count(
                            entry,
                            *total,
                            *last,
                            replayed,
                            &mut replay,
                            child,
                            fresh,
                            &mut pending,
                        );
                    }
                    // The count's own response comes last, after records it drained.
                    let since = self.ledger.responses[counted..]
                        .last()
                        .map_or(entry.at, |response| response.at);
                    let own = !(replay.active || replayed);
                    let reading = limits.as_ref().filter(|_| own).map(|limits| {
                        let birth = !self.state.born && self.thread.account.is_some();
                        self.state.born = true;
                        self.ledger.readings.push(Reading {
                            at: entry.at,
                            since,
                            asked: self.state.asked,
                            thread: self.index,
                            limits: Arc::clone(limits),
                            birth,
                        });
                        self.ledger.readings.len() - 1
                    });
                    if reading.is_some() {
                        self.state.reading = reading;
                    }
                    if own {
                        self.state.ask_after(entry.at);
                    }
                    // What this count added was served under these limits.
                    for response in &mut self.ledger.responses[counted..] {
                        response.reading = reading;
                    }
                }
            }
        }
        for record in pending {
            self.emit_record(record);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn count<'e>(
        &mut self,
        entry: &'e Entry,
        total: CodexTokens,
        last: Option<CodexTokens>,
        replayed: bool,
        replay: &mut Replay<'_>,
        child: bool,
        fresh: bool,
        pending: &mut Vec<&'e Entry>,
    ) {
        if replayed {
            self.state.prev = Some(total);
            return;
        }
        if replay.active && replay.start.is_none() {
            match replay.parent_snapshots {
                Some(snapshots) if snapshots.contains(&total) => {
                    self.state.prev = Some(total);
                    return;
                }
                Some(_) => replay.active = false,
                None if !replay.triggered => {
                    self.state.prev = Some(total);
                    return;
                }
                None => replay.active = false,
            }
        }
        let delta = match self.state.prev {
            Some(prev) if prev == total => None,
            // The counter went back (never seen in real logs): trust the
            // response's own usage instead.
            Some(prev) => Some(total.checked_sub(&prev).or(last).unwrap_or_default()),
            // The thread's first snapshot: everything it counted is its own.
            None if fresh && !child => Some(total),
            None => Some(last.unwrap_or(total)),
        };
        self.state.prev = Some(total);
        let Some(usage) = delta.filter(|usage| !usage.is_zero()) else {
            return;
        };
        // The latest records that add up to the advance are its responses;
        // the ones before them (compactions) the counter never included.
        let covering = (1..=pending.len()).find(|&count| {
            let mut sum = CodexTokens::default();
            for record in &pending[pending.len() - count..] {
                sum += record_usage(record).unwrap_or_default();
            }
            same_counts(&sum, &usage)
        });
        let covered = pending.split_off(pending.len() - covering.unwrap_or(0));
        for record in pending.drain(..) {
            self.emit_record(record);
        }
        if covered.is_empty() {
            let turn = self.state.turn.clone();
            self.emit(entry.at, turn, usage);
        }
        for record in covered {
            self.emit_record(record);
        }
    }

    fn emit_record(&mut self, entry: &Entry) {
        let Record::Usage {
            response,
            turn,
            usage,
        } = &entry.record
        else {
            return;
        };
        if !self.seen.insert(Arc::clone(response)) {
            return;
        }
        let turn = turn.clone().or_else(|| self.state.turn.clone());
        self.emit(entry.at, turn, *usage);
    }

    fn emit(&mut self, at: DateTime<Utc>, turn: Option<Arc<str>>, tokens: CodexTokens) {
        let state = &mut self.state;
        let (model, effort) = state
            .contexts
            .get(&turn)
            .cloned()
            .unwrap_or_else(|| (state.model.clone(), state.effort.clone()));
        let window = state.window;
        if !state.turns.contains(&turn) {
            self.ledger.turns.push(Turn {
                at,
                model: model.clone(),
                effort: effort.clone(),
            });
            state.turns.insert(turn);
            self.thread.turns += 1;
        }
        let cost = self.prices.cost(model.as_ref(), &tokens);
        let usage = CodexUsage::of(tokens, cost.map(|cost| cost.total()));
        let thread = &mut *self.thread;
        thread.usage += usage;
        let context = tokens.total();
        if context > thread.peak {
            thread.peak = context;
            thread.peak_window = window;
        }
        thread.model = model.clone();
        thread.effort = effort.clone();
        self.touch(at);
        self.ledger.total += usage;
        self.ledger.responses.push(Response {
            at,
            thread: self.index,
            tokens,
            cost,
            model,
            effort,
            window,
            reading: self.state.reading,
        });
    }

    fn touch(&mut self, at: DateTime<Utc>) {
        let last = &mut self.thread.last_activity;
        *last = Some(last.map_or(at, |last| last.max(at)));
    }
}

fn record_usage(entry: &Entry) -> Option<CodexTokens> {
    match entry.record {
        Record::Usage { usage, .. } => Some(usage),
        _ => None,
    }
}

/// Whether two counts are one response's, cache writes aside: a record may
/// log those and the thread's counter not.
fn same_counts(a: &CodexTokens, b: &CodexTokens) -> bool {
    let without_writes = |tokens: &CodexTokens| CodexTokens {
        cache_write: 0,
        ..*tokens
    };
    without_writes(a) == without_writes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, cached: u64, output: u64, reasoning: u64) -> CodexTokens {
        CodexTokens {
            input,
            cached,
            cache_write: 0,
            output,
            reasoning,
        }
    }

    /// Builds a rollout from compact JSON lines; `{t}` in a line becomes a
    /// timestamp a second per line after 10:00 plus `minute` minutes.
    fn rollout_at(minute: usize, lines: &[String]) -> FileLog {
        let mut log = FileLog::default();
        for (n, line) in lines.iter().enumerate() {
            let at = format!("2026-10-19T10:{:02}:{:02}.000Z", minute + n / 60, n % 60);
            log.ingest(line.replace("{t}", &at).as_bytes());
        }
        log
    }

    fn rollout(lines: &[String]) -> FileLog {
        rollout_at(0, lines)
    }

    fn meta(id: &str, extra: &str) -> String {
        format!(
            r#"{{"timestamp":"{{t}}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{{t}}"{extra}}}}}"#
        )
    }

    fn usage_json(t: CodexTokens) -> String {
        format!(
            r#"{{"input_tokens":{},"cached_input_tokens":{},"cache_write_input_tokens":{},"output_tokens":{},"reasoning_output_tokens":{},"total_tokens":{}}}"#,
            t.input,
            t.cached,
            t.cache_write,
            t.output,
            t.reasoning,
            t.total()
        )
    }

    fn count(total: CodexTokens, last: CodexTokens) -> String {
        format!(
            r#"{{"timestamp":"{{t}}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{},"last_token_usage":{},"model_context_window":258400}}}}}}"#,
            usage_json(total),
            usage_json(last)
        )
    }

    fn record(id: &str, usage: CodexTokens) -> String {
        format!(
            r#"{{"timestamp":"{{t}}","type":"token_usage_record","payload":{{"response_id":"{id}","turn_id":"turn-1","usage":{}}}}}"#,
            usage_json(usage)
        )
    }

    fn turn(id: &str, model: &str, effort: &str) -> [String; 2] {
        [
            format!(
                r#"{{"timestamp":"{{t}}","type":"event_msg","payload":{{"type":"task_started","turn_id":"{id}","model_context_window":258400}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{{t}}","type":"turn_context","payload":{{"turn_id":"{id}","model":"{model}","effort":"{effort}"}}}}"#
            ),
        ]
    }

    fn build(files: &[(&str, &FileLog)]) -> Ledger {
        Ledger::build(
            files.iter().map(|(name, log)| (*name, *log)),
            &Pricing::default(),
        )
    }

    #[test]
    fn counts_counter_advances_and_skips_repeated_snapshots() {
        let a = tokens(1000, 800, 50, 10);
        let b = tokens(1500, 1200, 70, 20);
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-1", "gpt-6-astra", "high"));
        lines.push(count(a, a));
        // After a compaction: same total, `last` holds only the context size.
        lines.push(count(a, tokens(0, 0, 0, 0)));
        lines.push(count(tokens(2500, 2000, 120, 30), b));
        let log = rollout(&lines);
        let ledger = build(&[("rollout-1.jsonl", &log)]);
        assert_eq!(ledger.responses.len(), 2);
        assert_eq!(ledger.total.tokens, tokens(2500, 2000, 120, 30));
        let thread = &ledger.threads[0];
        assert_eq!(thread.turns, 1);
        assert_eq!(thread.peak, 1570);
        assert_eq!(thread.peak_window, Some(258_400));
        assert_eq!(thread.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(thread.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_first_snapshot_counts_usage_logged_before_it() {
        // An aborted first turn never logged its own `token_count`, yet the
        // counter includes it: Codex's total for the thread does too.
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-2", "gpt-5.6-sol", "max"));
        lines.push(count(
            tokens(66_000, 40_000, 891, 0),
            tokens(24_000, 20_000, 499, 0),
        ));
        let log = rollout(&lines);
        let ledger = build(&[("rollout-1.jsonl", &log)]);
        assert_eq!(ledger.total.tokens.total(), 66_891);
    }

    #[test]
    fn usage_records_cover_advances_and_compactions_count_on_their_own() {
        let first = tokens(17_602, 12_288, 179, 0);
        let compact = tokens(219_369, 214_400, 4081, 0);
        let second = tokens(20_373, 17_408, 601, 201);
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-1", "gpt-6-astra", "max"));
        lines.push(record("resp_1", first));
        lines.push(count(first, first));
        lines.push(record("resp_c", compact));
        lines.push(
            r#"{"timestamp":"{t}","type":"compacted","payload":{"compaction_response_id":"resp_c"}}"#
                .into(),
        );
        lines.push(count(first, tokens(0, 0, 0, 0)));
        lines.push(record("resp_2", second));
        let total = tokens(37_975, 29_696, 780, 201);
        lines.push(count(total, second));
        let log = rollout(&lines);
        let ledger = build(&[("rollout-1.jsonl", &log)]);
        let mut expected = total;
        expected += compact;
        assert_eq!(ledger.total.tokens, expected);
        assert_eq!(ledger.responses.len(), 3);
        assert_eq!(ledger.threads[0].compactions, 1);
        assert_eq!(ledger.compactions.len(), 1);
        assert_eq!(ledger.threads[0].peak, compact.total());

        // The same records in an archived copy or another file count once.
        let copy = rollout(&lines);
        let mut other = vec![meta("t2", "")];
        other.push(record("resp_2", second));
        other.push(count(second, second));
        let other = rollout(&other);
        let ledger = build(&[("a.jsonl", &log), ("b.jsonl", &copy), ("c.jsonl", &other)]);
        assert_eq!(
            ledger.responses.len(),
            3,
            "copies of a thread merge as pages"
        );
    }

    #[test]
    fn a_record_covers_its_advance_whatever_cache_writes_it_logs() {
        // The counter may leave out cache writes that the response's record logs.
        let logged = CodexTokens {
            cache_write: 1024,
            ..tokens(20_373, 17_408, 601, 201)
        };
        let counted = CodexTokens {
            cache_write: 0,
            ..logged
        };
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-1", "gpt-6-astra", "high"));
        lines.push(record("resp_1", logged));
        lines.push(count(counted, counted));
        let ledger = build(&[("rollout-1.jsonl", &rollout(&lines))]);
        assert_eq!(ledger.responses.len(), 1, "counted once");
        assert_eq!(ledger.total.tokens, logged);
    }

    #[test]
    fn records_that_add_up_to_an_advance_all_cover_it() {
        // Two responses recorded before the one count that includes them both.
        let first = tokens(17_602, 12_288, 179, 0);
        let second = tokens(20_373, 17_408, 601, 201);
        let mut total = first;
        total += second;
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-1", "gpt-6-astra", "high"));
        lines.push(record("resp_1", first));
        lines.push(record("resp_2", second));
        lines.push(count(total, second));
        let ledger = build(&[("rollout-1.jsonl", &rollout(&lines))]);
        assert_eq!(ledger.responses.len(), 2);
        assert_eq!(ledger.total.tokens, total);
    }

    #[test]
    fn a_continuation_page_counts_only_its_advance() {
        let a = tokens(8_000_000, 7_000_000, 239_458, 50_000);
        let page1 = rollout(&[meta("t1", ""), count(a, a)]);
        let more = tokens(100_000, 90_000, 5_836, 0);
        let b = tokens(8_100_000, 7_090_000, 245_294, 50_000);
        let base = r#","history_base":{"thread_id":"t1","end_ordinal_exclusive":2}"#;
        let page2 = rollout(&[meta("t1", base), record("resp_9", more), count(b, more)]);
        let ledger = build(&[("rollout-2_w.jsonl", &page2), ("rollout-1.jsonl", &page1)]);
        assert_eq!(ledger.threads.len(), 1);
        assert_eq!(ledger.total.tokens, b);
        // Without its first page, the second still counts only its own response.
        let ledger = build(&[("rollout-2_w.jsonl", &page2)]);
        assert_eq!(ledger.total.tokens, more);
    }

    #[test]
    fn a_forked_child_skips_the_parent_snapshots_it_replays() {
        let p1 = tokens(12_000_000, 11_000_000, 50_000, 20_000);
        let p2 = tokens(12_018_000, 11_015_000, 51_000, 20_100);
        let parent = rollout(&[
            meta("parent", ""),
            count(p1, p1),
            count(p2, tokens(18_000, 15_000, 1000, 100)),
        ]);
        let own = tokens(19_000, 17_000, 293, 50);
        let mut c_total = p2;
        c_total += own;
        let fork = r#","forked_from_id":"parent","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}},"agent_nickname":"Fermat","subagent_history_start_ordinal":99"#;
        let child_lines = [
            meta("child", fork),
            r#"{"timestamp":"{t}","type":"compacted","payload":{"compaction_response_id":null}}"#.into(),
            count(p1, tokens(0, 0, 0, 0)),
            count(p2, tokens(18_000, 15_000, 1000, 100)),
            r#"{"timestamp":"{t}","type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}"#.into(),
            count(c_total, own),
        ];
        let child = rollout_at(5, &child_lines);
        let ledger = build(&[("p.jsonl", &parent), ("c.jsonl", &child)]);
        let child_thread = &ledger.threads[1];
        assert_eq!(&*child_thread.id, "child");
        assert_eq!(child_thread.parent, Some(0));
        assert_eq!(child_thread.root, 0);
        assert_eq!(
            child_thread.usage.tokens, own,
            "the replayed history is the parent's"
        );
        assert_eq!(child_thread.compactions, 0, "so is the replayed compaction");
        assert_eq!(ledger.threads[0].usage.tokens, p2);

        // With the parent's rollout gone, the trigger marker bounds the replay.
        let ledger = build(&[("c.jsonl", &child)]);
        assert_eq!(ledger.threads[0].usage.tokens, own);
        assert_eq!(ledger.threads[0].parent, None);
    }

    #[test]
    fn ordinary_forks_stay_top_level_and_own_their_subagents() {
        use crate::codex::paths::CodexHome;
        use crate::codex::report::{Titles, report};
        use crate::codex::signin::SignIns;

        let a = tokens(1000, 0, 10, 0);
        let b = tokens(700, 0, 7, 0);
        let parent = rollout(&[meta("parent", ""), count(a, a)]);
        let mut total = a;
        total += b;
        let fork = rollout_at(
            1,
            &[
                meta("fork", r#","forked_from_id":"parent""#),
                count(a, a),
                count(total, b),
            ],
        );
        let ledger = build(&[("p.jsonl", &parent), ("f.jsonl", &fork)]);
        assert_eq!(ledger.threads[1].parent, Some(0));
        assert!(!ledger.threads[1].subagent);
        assert_eq!(ledger.threads[1].root, 1);
        assert_eq!(ledger.threads[1].usage.tokens, b);
        assert_eq!(ledger.total.tokens, total);

        let home = CodexHome::from(std::path::PathBuf::from("/codex"));
        let now = "2026-10-19T11:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let data = report(
            &ledger,
            &Titles::new(),
            &home,
            &SignIns::default(),
            now,
            &Utc,
        );
        assert_eq!((data.today.threads, data.today.subagents), (2, 0));
        assert_eq!(data.threads.len(), 2);
        assert!(data.threads.iter().any(|row| row.id == "parent"));
        assert!(data.threads.iter().any(|row| row.id == "fork"));
        assert!(data.threads.iter().all(|row| row.subagent_share == 0.0));
        assert_eq!(data.context.subagents, 0);

        let c = tokens(300, 0, 3, 0);
        let sub = rollout_at(
            2,
            &[
                meta(
                    "sub",
                    r#","source":{"subagent":{"thread_spawn":{"parent_thread_id":"fork"}}}"#,
                ),
                count(c, c),
            ],
        );
        let ledger = build(&[("p.jsonl", &parent), ("f.jsonl", &fork), ("s.jsonl", &sub)]);
        assert_eq!(ledger.threads[2].root, 1);
        let data = report(
            &ledger,
            &Titles::new(),
            &home,
            &SignIns::default(),
            now,
            &Utc,
        );
        assert_eq!((data.today.threads, data.today.subagents), (2, 1));
        let row = data.threads.iter().find(|row| row.id == "fork").unwrap();
        assert_eq!(row.usage.tokens.total(), b.total() + c.total());
        assert_eq!(row.subagents, 1);
    }

    #[test]
    fn a_history_start_ordinal_bounds_the_replay_when_records_follow_it() {
        let own = tokens(17_501, 15_000, 400, 100);
        let fork = r#","forked_from_id":"parent","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}},"subagent_history_start_ordinal":3"#;
        let lines = [
            meta("child", fork).replace(r#""type""#, r#""ordinal":0,"type""#),
            r#"{"timestamp":"{t}","ordinal":1,"type":"event_msg","payload":{"type":"task_started","turn_id":"parent-turn"}}"#.into(),
            r#"{"timestamp":"{t}","ordinal":2,"type":"compacted","payload":{"compaction_response_id":null}}"#.into(),
            r#"{"timestamp":"{t}","ordinal":3,"type":"event_msg","payload":{"type":"task_started","turn_id":"own-turn"}}"#.into(),
            record("resp_own", own).replace(r#""type""#, r#""ordinal":4,"type""#),
            count(own, own).replace(r#""type":"event_msg""#, r#""ordinal":5,"type":"event_msg""#),
        ];
        let log = rollout(&lines);
        let ledger = build(&[("c.jsonl", &log)]);
        assert_eq!(ledger.total.tokens, own);
        assert_eq!(ledger.turns.len(), 1);
        assert_eq!(ledger.threads[0].compactions, 0);
    }

    #[test]
    fn a_history_start_past_the_end_says_nothing() {
        // Old subagent rollouts were migrated with the start set to their length.
        let own = tokens(15_000, 12_000, 590, 0);
        let spawn = r#","parent_thread_id":"gone","source":{"subagent":{"thread_spawn":{"parent_thread_id":"gone"}}},"subagent_history_start_ordinal":2"#;
        let lines = [meta("child", spawn), count(own, own)];
        let ledger = build(&[("c.jsonl", &rollout(&lines))]);
        assert_eq!(ledger.total.tokens, own);
    }

    #[test]
    fn a_reading_dates_the_response_it_leaves_out() {
        let with_limits = |count: String| {
            count.replace(
                "258400}}}",
                r#"258400},"rate_limits":{"limit_id":"codex","primary":{"used_percent":6.0,"window_minutes":300,"resets_at":1792414073}}}}"#,
            )
        };
        let first = tokens(17_602, 12_288, 179, 0);
        let second = tokens(20_373, 17_408, 601, 201);
        let mut total = first;
        total += second;
        let lines = [
            meta("t1", ""),
            record("resp_1", first),
            // The tool the response called ran before its count was logged.
            r#"{"timestamp":"{t}","type":"response_item","payload":{"type":"custom_tool_call_output"}}"#.into(),
            with_limits(count(first, first)),
            // Limits from a request still running: no response of their own yet.
            with_limits(count(first, first)),
            // Before Codex 0.153 a response is the counter's advance, logged with its count.
            with_limits(count(total, second)),
        ];
        let ledger = build(&[("rollout-1.jsonl", &rollout(&lines))]);
        let time = |second: u32| {
            format!("2026-10-19T10:00:{second:02}Z")
                .parse::<DateTime<Utc>>()
                .unwrap()
        };
        let readings: Vec<_> = ledger
            .readings
            .iter()
            .map(|reading| (reading.at, reading.since))
            .collect();
        assert_eq!(
            readings,
            [(time(3), time(1)), (time(4), time(4)), (time(5), time(5))]
        );
    }

    #[test]
    fn subagents_hang_under_their_root_and_turns_need_usage() {
        let a = tokens(1000, 0, 10, 0);
        let mut root = vec![meta("root", "")];
        root.extend(turn("r-1", "gpt-6.1-sol", "ultra"));
        root.push(count(a, a));
        root.extend(turn("r-2", "gpt-6.1-sol", "ultra"));
        let spawn = |id: &str, parent: &str| {
            format!(
                r#","parent_thread_id":"{parent}","source":{{"subagent":{{"thread_spawn":{{"parent_thread_id":"{parent}"}}}}}}"#
            )
            .replace("{id}", id)
        };
        let b = tokens(700, 0, 7, 0);
        let c = tokens(300, 0, 3, 0);
        let sub = rollout_at(1, &[meta("sub", &spawn("sub", "root")), count(b, b)]);
        let subsub = rollout_at(2, &[meta("subsub", &spawn("subsub", "sub")), count(c, c)]);
        let root = rollout(&root);
        let ledger = build(&[("r.jsonl", &root), ("s.jsonl", &sub), ("ss.jsonl", &subsub)]);
        assert_eq!(
            ledger.threads.iter().map(|t| t.root).collect::<Vec<_>>(),
            [0, 0, 0]
        );
        assert_eq!(
            ledger.threads[0].turns, 1,
            "a turn without a response does not count"
        );
        assert_eq!(ledger.turns.len(), 3);
        let mut total = a;
        total += b;
        total += c;
        assert_eq!(ledger.total.tokens, total);
    }

    #[test]
    fn responses_are_priced_one_by_one_at_their_turns_model() {
        // Two Astra responses of 200K input: past 272K together, not apart.
        let first = tokens(200_000, 150_000, 1_000, 200);
        let second = tokens(200_000, 190_000, 2_000, 500);
        let mut lines = vec![meta("t1", "")];
        lines.extend(turn("turn-1", "gpt-6-astra", "high"));
        lines.push(count(first, first));
        let mut total = first;
        total += second;
        lines.push(count(total, second));
        // A model without a price: its tokens count, its cost does not.
        lines.extend(turn("turn-2", "gpt-reserve", "high"));
        let reserve = tokens(10_000, 0, 100, 0);
        total += reserve;
        lines.push(count(total, reserve));
        let log = rollout(&lines);
        let ledger = build(&[("rollout-1.jsonl", &log)]);
        // 50K × $10 + 150K × $1 + 1K × $50, then 10K × $10 + 190K × $1 + 2K × $50.
        let astra = (0.5 + 0.15 + 0.05) + (0.1 + 0.19 + 0.1);
        assert!((ledger.total.cost_usd - astra).abs() < 1e-9);
        assert_eq!(ledger.total.unpriced, 1);
        assert_eq!(ledger.total.tokens, total);
        assert_eq!(ledger.threads[0].usage, ledger.total);
        let priced: Vec<_> = ledger.responses.iter().map(|r| r.cost.is_some()).collect();
        assert_eq!(priced, [true, true, false]);
        assert_eq!(ledger.custom_prices, 0);

        // A local price for Astra: $1 for every token of its two responses.
        let pricing =
            Pricing::parse(r#"{ "gpt-6-astra": { "input": 1, "output": 1, "cacheRead": 1 } }"#)
                .unwrap();
        let ledger = Ledger::build([("rollout-1.jsonl", &log)], &pricing);
        assert!((ledger.total.cost_usd - 0.403).abs() < 1e-9);
        assert_eq!(ledger.custom_prices, 1);
    }
}
