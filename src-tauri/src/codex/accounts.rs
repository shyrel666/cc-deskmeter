//! Whose limits each reading shows, and each account's windows from them.
//!
//! Codex logs the signed-in account's limits with every response but names
//! no account. `creator_account_id` only says who started a thread: a thread
//! continued after switching accounts reports the other account's limits
//! (468 of 3,758 readings in the author's logs). A window, though, belongs to
//! one account, so readings that share a window (same length, same reset,
//! logged with up to half a minute of jitter) share an account; and a new
//! top-level thread's first reading was taken under the account that started
//! it. On the author's logs every window since accounts were logged (Codex
//! 0.158) was reached that way, without a conflict. A window no new thread
//! reaches, as when the limits reset early and the user carries on in an
//! old thread, takes the account `auth.json` named when its requests were
//! sent (`signin`); guessing from the account before would have been wrong
//! for 9 of the author's 18 windows. Windows neither reaches stay
//! unattributed.
//!
//! A reading also lags a response behind: the limits logged with a response
//! were read before that response was charged (every window's first reading
//! shows 0%), so the latest reading misses the latest response, two points
//! of the 5-hour window for a long-context Astra one. A window therefore
//! shows its highest reading plus the responses logged since, at the points
//! per dollar of API-equivalent use that the plan's latest windows gained
//! (`rates`). Replayed over the author's logs (3,665 readings), that matches
//! the next 5-hour reading 69% of the time against 26% for the reading
//! alone, and is two points or more off for 2.5% against 33%. A response
//! moves the weekly window a fraction of a point: it reads alike either way.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::ledger::{Ledger, Response};
use super::parser::{Limits, WindowReading};
use super::signin::SignIn;
use super::types::CodexWindow;

/// Readings with compatible accounts whose resets are this close share a
/// window: Codex logs its reset with a few seconds' jitter (32 s at most seen).
const SAME_RESET_SECONDS: i64 = 60;
/// Windows shorter than this are the short (5-hour) one, longer the weekly.
const SHORT_WINDOW_MINUTES: u64 = 24 * 60;
/// Windows pooled for a plan's rate, the latest ones (the current included).
const RATE_WINDOWS: usize = 20;
/// A window that reached this stops rising at the limit while responses can
/// still be logged, so it says nothing of a rate.
const SATURATED: f64 = 99.0;

pub const SHORT: usize = 0;
pub const LONG: usize = 1;

/// Readings that name an account, and when the first was logged.
type Vote = (u64, DateTime<Utc>);
/// Per plan: points per dollar of its short and long windows (see `rates`).
type Rates = Vec<(Option<Arc<str>>, [Option<f64>; 2])>;

pub struct Accounts {
    /// Every account that owns a window, in the order of its first reading.
    pub ids: Vec<Arc<str>>,
    /// Per reading (index into `Ledger::readings`): index into `ids`.
    owner: Vec<Option<usize>>,
    /// Per reading: its short and long window, numbered in the order of their resets.
    windows: Vec<[Option<usize>; 2]>,
    /// Per reading: the set of readings that share its windows (one account's).
    sets: Vec<usize>,
    rates: Rates,
}

/// The plan's own limits (`codex`); other buckets (`premium`) carry no windows.
pub fn plan_limits(limits: &Limits) -> bool {
    limits.limit_id.as_deref().is_none_or(|id| id == "codex")
}

/// The short and long windows of a reading, by their length.
pub fn windows(limits: &Limits) -> [Option<&WindowReading>; 2] {
    let mut slots = [None, None];
    for window in [&limits.primary, &limits.secondary].into_iter().flatten() {
        let slot = usize::from(window.window_minutes >= SHORT_WINDOW_MINUTES);
        slots[slot].get_or_insert(window);
    }
    slots
}

impl Accounts {
    pub fn attribute(ledger: &Ledger, signins: &[SignIn]) -> Self {
        let readings = &ledger.readings;
        let count = readings.len();
        let mut numbers = vec![[None, None]; count];
        let mut sets = Sets::new(ledger);
        for kind in [SHORT, LONG] {
            let mut resets: Vec<(i64, usize)> = readings
                .iter()
                .enumerate()
                .filter(|(_, reading)| plan_limits(&reading.limits))
                .filter_map(|(index, reading)| {
                    let resets_at = windows(&reading.limits)[kind]?.resets_at?;
                    Some((resets_at.timestamp(), index))
                })
                .collect();
            resets.sort_unstable();
            let mut window = 0;
            // Open clusters: last reset, reading and window number.
            let mut open: Vec<(i64, usize, usize)> = Vec::new();
            for (reset, index) in resets {
                open.retain(|&(previous, _, _)| reset - previous <= SAME_RESET_SECONDS);
                let account = sets.account(index);
                let closest = open
                    .iter()
                    .enumerate()
                    .filter_map(|(position, &(previous, other, _))| {
                        let known = sets.account(other);
                        (account.is_none() || known.is_none() || account == known)
                            .then_some((previous, position))
                    })
                    .max()
                    .map(|(_, position)| position);
                let number = match closest {
                    Some(position) => {
                        let (_, other, number) = open[position];
                        sets.union(index, other);
                        open[position] = (reset, index, number);
                        number
                    }
                    None => {
                        let number = window;
                        window += 1;
                        open.push((reset, index, number));
                        number
                    }
                };
                numbers[index][kind] = Some(number);
            }
        }

        // A new thread's first reading names the account behind its window.
        // Per set of readings: per account, its votes and the first of them.
        let mut votes: HashMap<usize, HashMap<&Arc<str>, Vote>> = HashMap::new();
        for (index, reading) in readings.iter().enumerate() {
            let Some(account) = ledger.threads[reading.thread].account.as_ref() else {
                continue;
            };
            if reading.birth && numbers[index] != [None, None] {
                vote(&mut votes, sets.find(index), account, reading.at);
            }
        }
        let mut winners = elect(votes);
        // Else the sign-in its requests certainly went out under.
        let mut votes: HashMap<usize, HashMap<&Arc<str>, Vote>> = HashMap::new();
        for (index, reading) in readings.iter().enumerate() {
            let Some(asked) = reading.asked else {
                continue;
            };
            let set = sets.find(index);
            if numbers[index] == [None, None] || winners.contains_key(&set) {
                continue;
            }
            if let Some(signin) = signins
                .iter()
                .rev()
                .find(|signin| signin.covers(asked, reading.at))
            {
                vote(&mut votes, set, &signin.account, reading.at);
            }
        }
        winners.extend(elect(votes));

        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&index| (readings[index].at, index));
        let mut ids: Vec<Arc<str>> = Vec::new();
        let mut owner = vec![None; count];
        for index in order {
            let Some(&account) = winners.get(&sets.find(index)) else {
                continue;
            };
            let id = match ids.iter().position(|id| id == account) {
                Some(id) => id,
                None => {
                    ids.push(Arc::clone(account));
                    ids.len() - 1
                }
            };
            owner[index] = Some(id);
        }
        Self {
            ids,
            owner,
            rates: rates(ledger, &numbers),
            windows: numbers,
            sets: (0..count).map(|index| sets.find(index)).collect(),
        }
    }

    /// Points of a `kind` window per dollar on `plan`, once its windows tell.
    fn rate(&self, plan: Option<&str>, kind: usize) -> Option<f64> {
        self.rates
            .iter()
            .find(|(known, _)| known.as_deref() == plan)?
            .1[kind]
    }

    pub fn of_reading(&self, reading: usize) -> Option<usize> {
        self.owner[reading]
    }

    /// The account a response was served under, by the reading logged with it.
    pub fn of_response(&self, response: &Response) -> Option<usize> {
        response.reading.and_then(|reading| self.owner[reading])
    }

    /// The window of `kind` a response was served in, by the reading logged
    /// with it: its number among the windows of that length.
    pub fn window_of(&self, response: &Response, kind: usize) -> Option<usize> {
        response
            .reading
            .and_then(|reading| self.windows[reading][kind])
    }

    /// Walks an account's readings forward in time (see `Tracker`).
    pub fn tracker<'l>(&'l self, ledger: &'l Ledger, account: usize) -> Tracker<'l> {
        self.tracker_of(ledger, |index| self.owner[index] == Some(account))
    }

    /// Walks the readings that share windows with `reading`: whoever's they
    /// are, they are one account's.
    pub fn tracker_with<'l>(&'l self, ledger: &'l Ledger, reading: usize) -> Tracker<'l> {
        self.tracker_of(ledger, |index| self.sets[index] == self.sets[reading])
    }

    fn tracker_of<'l>(&'l self, ledger: &'l Ledger, keep: impl Fn(usize) -> bool) -> Tracker<'l> {
        let mut readings: Vec<usize> = (0..ledger.readings.len())
            .filter(|&index| keep(index))
            .collect();
        readings.sort_by_key(|&index| (ledger.readings[index].at, index));
        let mut responses: Vec<usize> = (0..ledger.responses.len())
            .filter(|&index| ledger.responses[index].reading.is_some_and(&keep))
            .collect();
        responses.sort_by_key(|&index| (ledger.responses[index].at, index));
        Tracker {
            accounts: self,
            ledger,
            readings,
            next: 0,
            responses,
            next_response: 0,
            windows: [None, None],
            plan: None,
        }
    }
}

/// One more vote for `account` behind `set`.
fn vote<'a>(
    votes: &mut HashMap<usize, HashMap<&'a Arc<str>, Vote>>,
    set: usize,
    account: &'a Arc<str>,
    at: DateTime<Utc>,
) {
    let vote = votes
        .entry(set)
        .or_default()
        .entry(account)
        .or_insert((0, at));
    vote.0 += 1;
    vote.1 = vote.1.min(at);
}

/// Per set of readings, the account with the most votes, then the earliest.
fn elect(votes: HashMap<usize, HashMap<&Arc<str>, Vote>>) -> HashMap<usize, &Arc<str>> {
    votes
        .into_iter()
        .filter_map(|(set, votes)| {
            let winner =
                votes
                    .into_iter()
                    .max_by(|(a, (a_votes, a_first)), (b, (b_votes, b_first))| {
                        a_votes
                            .cmp(b_votes)
                            .then_with(|| b_first.cmp(a_first))
                            .then_with(|| b.cmp(a))
                    })?;
            Some((set, winner.0))
        })
        .collect()
}

/// Points per dollar of API-equivalent use, per plan and kind of window:
/// what each of the plan's latest windows gained from its first reading to
/// its highest, over what the responses that gain covers cost, pooled.
/// Windows that saturated, or that served a model without a price, are left
/// out. On the author's Plus logs a dollar took 8.4 points of the 5-hour
/// window and 1.2 of the week.
fn rates(ledger: &Ledger, numbers: &[[Option<usize>; 2]]) -> Rates {
    /// Use and `since` of a reading.
    type Point = (f64, DateTime<Utc>);
    #[derive(Clone)]
    struct Span<'l> {
        first: Point,
        /// The highest reading, the latest of equals.
        top: Point,
        plan: Option<&'l Arc<str>>,
        /// What the responses from `first` to `top` cost; `None` if one has no price.
        cost: Option<f64>,
    }

    let mut order: Vec<usize> = (0..ledger.readings.len()).collect();
    order.sort_by_key(|&index| (ledger.readings[index].at, index));
    let mut rates = Rates::new();
    for kind in [SHORT, LONG] {
        let mut spans: Vec<Option<Span>> = Vec::new();
        for &index in &order {
            let reading = &ledger.readings[index];
            let (Some(number), Some(window)) =
                (numbers[index][kind], windows(&reading.limits)[kind])
            else {
                continue;
            };
            if spans.len() <= number {
                spans.resize(number + 1, None);
            }
            let point = (window.used_percent, reading.since);
            match &mut spans[number] {
                Some(span) if point.0 >= span.top.0 => span.top = point,
                Some(_) => {}
                slot => {
                    *slot = Some(Span {
                        first: point,
                        top: point,
                        plan: reading.limits.plan.as_ref(),
                        cost: Some(0.0),
                    })
                }
            }
        }
        for response in &ledger.responses {
            let Some(number) = response.reading.and_then(|reading| numbers[reading][kind]) else {
                continue;
            };
            if let Some(span) = &mut spans[number]
                && (span.first.1..span.top.1).contains(&response.at)
            {
                span.cost = span
                    .cost
                    .zip(response.cost)
                    .map(|(sum, cost)| sum + cost.total());
            }
        }
        // Per plan: windows pooled, points and cost.
        let mut pools: Vec<(Option<&Arc<str>>, usize, f64, f64)> = Vec::new();
        for span in spans.iter().rev().flatten() {
            let Some(cost) = span.cost.filter(|cost| *cost > 0.0) else {
                continue;
            };
            if span.top.0 >= SATURATED {
                continue;
            }
            let pool = match pools.iter().position(|pool| pool.0 == span.plan) {
                Some(position) => &mut pools[position],
                None => {
                    pools.push((span.plan, 0, 0.0, 0.0));
                    pools.last_mut().expect("just pushed")
                }
            };
            if pool.1 < RATE_WINDOWS {
                pool.1 += 1;
                pool.2 += span.top.0 - span.first.0;
                pool.3 += cost;
            }
        }
        for (plan, _, points, cost) in pools {
            let slot = match rates.iter().position(|(known, _)| known.as_ref() == plan) {
                Some(position) => &mut rates[position].1,
                None => {
                    rates.push((plan.cloned(), [None, None]));
                    &mut rates.last_mut().expect("just pushed").1
                }
            };
            slot[kind] = Some(points / cost);
        }
    }
    rates
}

/// One window of an account as its readings and responses so far show it.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    /// The window's number among windows of its length, by reset.
    window: usize,
    /// The most any reading of the window has shown: use within a window
    /// never goes down, so a lower reading is a stale one, logged late by a
    /// response that started earlier (a 1-point dip between concurrent threads).
    used_percent: f64,
    /// The `since` of that reading (the latest of equals).
    since: DateTime<Utc>,
    /// What the window's responses logged since cost: no reading shows them yet.
    pending: f64,
    /// Points per dollar of the reading's plan, once known.
    rate: Option<f64>,
    minutes: u64,
    resets_at: Option<DateTime<Utc>>,
    observed_at: DateTime<Utc>,
}

impl WindowState {
    /// The window's number (see `Accounts::window_of`) while it lasts at `now`.
    pub fn current(&self, now: DateTime<Utc>) -> Option<usize> {
        let reset = self.resets_at.is_some_and(|resets_at| resets_at <= now);
        (!reset).then_some(self.window)
    }

    /// The window as of `now`: its highest reading and what the responses
    /// since add, up to the limit. Once its reset has passed it starts over
    /// empty, and the next one starts with the next request.
    pub fn at(&self, now: DateTime<Utc>) -> CodexWindow {
        let reset = self.resets_at.is_some_and(|resets_at| resets_at <= now);
        let unshown = self.rate.map_or(0.0, |rate| rate * self.pending);
        let used = self.used_percent + unshown.min((100.0 - self.used_percent).max(0.0));
        CodexWindow {
            used_percent: if reset { 0.0 } else { used },
            window_minutes: self.minutes,
            resets_at: if reset { None } else { self.resets_at },
            observed_at: self.observed_at,
        }
    }
}

/// An account's windows and plan as of a moving point in time.
pub struct Tracker<'l> {
    accounts: &'l Accounts,
    ledger: &'l Ledger,
    /// The account's readings, by time.
    readings: Vec<usize>,
    next: usize,
    /// The responses served under those readings, by time.
    responses: Vec<usize>,
    next_response: usize,
    pub windows: [Option<WindowState>; 2],
    pub plan: Option<&'l str>,
}

impl Tracker<'_> {
    /// Takes in every reading and response logged up to `time`, in the
    /// order they were logged.
    pub fn advance_to(&mut self, time: DateTime<Utc>) {
        let ledger = self.ledger;
        loop {
            let reading = self
                .readings
                .get(self.next)
                .map(|&index| (ledger.readings[index].at, index))
                .filter(|(at, _)| *at <= time);
            let response = self
                .responses
                .get(self.next_response)
                .map(|&index| (ledger.responses[index].at, index))
                .filter(|(at, _)| *at <= time);
            match (reading, response) {
                (Some((reading_at, _)), Some((at, index))) if at <= reading_at => {
                    self.take_response(index)
                }
                (None, Some((_, index))) => self.take_response(index),
                (Some((_, index)), _) => self.take_reading(index),
                (None, None) => break,
            }
        }
    }

    fn take_response(&mut self, index: usize) {
        self.next_response += 1;
        let response = &self.ledger.responses[index];
        for (kind, state) in self.windows.iter_mut().enumerate() {
            if let Some(state) = state
                && self.accounts.window_of(response, kind) == Some(state.window)
                && response.at >= state.since
            {
                state.pending += response.cost.map_or(0.0, |cost| cost.total());
            }
        }
    }

    fn take_reading(&mut self, index: usize) {
        self.next += 1;
        let reading = &self.ledger.readings[index];
        let plan = reading.limits.plan.as_deref();
        if plan.is_some() {
            self.plan = plan;
        }
        let numbers = self.accounts.windows[index];
        // A reading of an earlier window than the one followed was logged late.
        let stale = self.windows.iter().zip(numbers).any(|(state, number)| {
            state
                .as_ref()
                .zip(number)
                .is_some_and(|(state, number)| number < state.window)
        });
        for (kind, window) in windows(&reading.limits).into_iter().enumerate() {
            // A window the plan's limits no longer carry is gone, not idle:
            // from July to August 2026 Plus had a weekly window only.
            if window.is_none() && !stale && plan_limits(&reading.limits) {
                self.windows[kind] = None;
            }
            let (Some(window), Some(number)) = (window, numbers[kind]) else {
                continue;
            };
            let pending = self.pending(kind, number, reading.since);
            let rate = self.accounts.rate(plan, kind);
            let state = &mut self.windows[kind];
            match state {
                // An earlier window: a stale reading.
                Some(current) if number < current.window => {}
                Some(current) if number == current.window => {
                    if window.used_percent >= current.used_percent {
                        current.used_percent = window.used_percent;
                        current.since = reading.since;
                        current.pending = pending;
                        current.rate = rate;
                    }
                    current.observed_at = reading.at;
                    current.resets_at = window.resets_at.or(current.resets_at);
                }
                _ => {
                    *state = Some(WindowState {
                        window: number,
                        used_percent: window.used_percent.max(0.0),
                        since: reading.since,
                        pending,
                        rate,
                        minutes: window.window_minutes,
                        resets_at: window.resets_at,
                        observed_at: reading.at,
                    })
                }
            }
        }
    }

    /// What the responses taken in so far that were logged from `since` on,
    /// in window `number` of `kind`, cost.
    fn pending(&self, kind: usize, number: usize, since: DateTime<Utc>) -> f64 {
        self.responses[..self.next_response]
            .iter()
            .rev()
            .map(|&index| &self.ledger.responses[index])
            .take_while(|response| response.at >= since)
            .filter(|response| self.accounts.window_of(response, kind) == Some(number))
            .filter_map(|response| response.cost)
            .map(|cost| cost.total())
            .sum()
    }
}

/// Disjoint sets over readings, each with the account its births name.
struct Sets<'l> {
    parents: Vec<usize>,
    accounts: Vec<Option<&'l str>>,
}

impl<'l> Sets<'l> {
    fn new(ledger: &'l Ledger) -> Self {
        Self {
            parents: (0..ledger.readings.len()).collect(),
            accounts: ledger
                .readings
                .iter()
                .map(|reading| {
                    if reading.birth {
                        ledger.threads[reading.thread].account.as_deref()
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }

    fn find(&mut self, mut index: usize) -> usize {
        while self.parents[index] != index {
            self.parents[index] = self.parents[self.parents[index]];
            index = self.parents[index];
        }
        index
    }

    fn account(&mut self, index: usize) -> Option<&'l str> {
        let root = self.find(index);
        self.accounts[root]
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.accounts[b] = self.accounts[b].or(self.accounts[a]);
        self.parents[a] = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::ledger::{Reading, Thread};
    use crate::codex::types::{CodexCostSplit, CodexTokens, CodexUsage};

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_792_000_000 + minute * 60, 0).unwrap()
    }

    fn thread(account: &str) -> Thread {
        Thread {
            id: account.into(),
            parent: None,
            root: 0,
            subagent: false,
            account: Some(account.into()),
            cwd: None,
            nickname: None,
            first_prompt: None,
            usage: CodexUsage::default(),
            turns: 0,
            compactions: 0,
            peak: 0,
            peak_window: None,
            model: None,
            effort: None,
            last_activity: None,
        }
    }

    /// A reading at `minute` of thread `thread`: 5-hour and weekly use and resets.
    fn reading(
        minute: i64,
        thread: usize,
        birth: bool,
        short: (f64, i64),
        long: (f64, i64),
    ) -> Reading {
        let window = |(used_percent, reset): (f64, i64), minutes| WindowReading {
            used_percent,
            window_minutes: minutes,
            resets_at: Some(at(reset)),
        };
        Reading {
            at: at(minute),
            since: at(minute),
            asked: Some(at(minute)),
            thread,
            birth,
            limits: Arc::new(Limits {
                limit_id: Some("codex".into()),
                plan: Some("plus".into()),
                primary: Some(window(short, 300)),
                secondary: Some(window(long, 10080)),
            }),
        }
    }

    /// A response of thread `thread` at `minute`, logged with reading
    /// `reading`, that would cost `usd` through the API.
    fn response(minute: i64, thread: usize, reading: usize, usd: Option<f64>) -> Response {
        Response {
            at: at(minute),
            thread,
            tokens: CodexTokens::default(),
            cost: usd.map(|usd| CodexCostSplit {
                input: usd,
                ..CodexCostSplit::default()
            }),
            model: None,
            effort: None,
            window: None,
            reading: Some(reading),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn windows_are_told_apart_by_length() {
        // From July to August 2026 the plan had a weekly window only, logged as `primary`.
        let reading = Limits {
            limit_id: Some("codex".into()),
            plan: None,
            primary: Some(WindowReading {
                used_percent: 12.0,
                window_minutes: 10080,
                resets_at: None,
            }),
            secondary: None,
        };
        let [short, long] = windows(&reading);
        assert!(short.is_none());
        assert_eq!(long.unwrap().used_percent, 12.0);
        assert!(plan_limits(&reading));
        assert!(!plan_limits(&Limits {
            limit_id: Some("premium".into()),
            ..reading
        }));
    }

    #[test]
    fn a_continued_thread_reports_the_account_signed_in_now() {
        let ledger = Ledger {
            threads: vec![thread("a"), thread("b")],
            readings: vec![
                // Thread 0 starts under account a (weekly window resetting at 10_000).
                reading(0, 0, true, (10.0, 300), (40.0, 10_000)),
                // Thread 1 starts under b (its own windows).
                reading(5, 1, true, (3.0, 305), (7.0, 12_000)),
                // The user switches back to b and continues thread 0: b's windows,
                // logged with a few seconds' jitter on the reset.
                reading(10, 0, false, (4.0, 305), (8.0, 12_000)),
                // A stale reading of a's window, lower than it already was.
                reading(11, 1, false, (9.0, 300), (39.0, 10_000)),
            ],
            ..Ledger::default()
        };
        let accounts = Accounts::attribute(&ledger, &[]);
        assert_eq!(&*accounts.ids[0], "a");
        assert_eq!(&*accounts.ids[1], "b");
        let owners: Vec<_> = (0..4).map(|i| accounts.of_reading(i)).collect();
        assert_eq!(owners, [Some(0), Some(1), Some(1), Some(0)]);

        let mut a = accounts.tracker(&ledger, 0);
        a.advance_to(at(20));
        let weekly = a.windows[LONG].as_ref().unwrap().at(at(20));
        assert_eq!(weekly.used_percent, 40.0, "the stale 39% does not lower it");
        assert_eq!(a.plan, Some("plus"));
        let five = a.windows[SHORT].as_ref().unwrap().at(at(400));
        assert_eq!(
            (five.used_percent, five.resets_at),
            (0.0, None),
            "reset since"
        );
        let mut b = accounts.tracker(&ledger, 1);
        b.advance_to(at(20));
        assert_eq!(
            b.windows[LONG].as_ref().unwrap().at(at(20)).used_percent,
            8.0
        );
    }

    #[test]
    fn nearby_resets_keep_different_birth_accounts_apart() {
        let week = |minute, thread, birth, short, seconds| {
            let mut reading = reading(minute, thread, birth, (10.0, short), (20.0, 10_000));
            Arc::make_mut(&mut reading.limits)
                .secondary
                .as_mut()
                .unwrap()
                .resets_at = Some(at(10_000) + chrono::TimeDelta::seconds(seconds));
            reading
        };
        let ledger = Ledger {
            threads: vec![thread("a"), thread("b"), thread("a")],
            readings: vec![
                week(0, 0, true, 300, 0),
                week(1, 1, true, 400, 30),
                // The short windows already name these sets; the closest
                // weekly reset belongs to the other account.
                week(2, 0, false, 300, 50),
                week(3, 1, false, 400, 70),
                // Without a birth or shared short window, take the nearest.
                week(4, 0, false, 500, 90),
                week(5, 2, true, 600, 110),
            ],
            ..Ledger::default()
        };
        let accounts = Accounts::attribute(&ledger, &[]);
        assert_eq!(accounts.ids, [Arc::<str>::from("a"), Arc::<str>::from("b")]);
        let owners: Vec<_> = (0..6).map(|index| accounts.of_reading(index)).collect();
        assert_eq!(
            owners,
            [Some(0), Some(1), Some(0), Some(1), Some(1), Some(0)]
        );
        let windows: Vec<_> = (0..6)
            .map(|index| accounts.window_of(&response(6, 0, index, None), LONG))
            .collect();
        assert_eq!(
            windows,
            [Some(0), Some(1), Some(0), Some(1), Some(1), Some(0)]
        );
    }

    #[test]
    fn a_window_no_new_thread_reaches_stays_unattributed() {
        let ledger = Ledger {
            threads: vec![thread("a")],
            readings: vec![
                reading(0, 0, true, (10.0, 300), (40.0, 10_000)),
                // Another account's windows, seen only in an old thread of a.
                reading(10, 0, false, (1.0, 310), (2.0, 20_000)),
            ],
            ..Ledger::default()
        };
        let accounts = Accounts::attribute(&ledger, &[]);
        assert_eq!(accounts.ids.len(), 1);
        assert_eq!(accounts.of_reading(0), Some(0));
        assert_eq!(accounts.of_reading(1), None);
        // Its windows are still known, if not whose they are.
        let mut current = accounts.tracker_with(&ledger, 1);
        current.advance_to(at(20));
        assert_eq!(
            current.windows[LONG]
                .as_ref()
                .unwrap()
                .at(at(20))
                .used_percent,
            2.0
        );
        assert_eq!(
            current.windows[SHORT]
                .as_ref()
                .unwrap()
                .at(at(20))
                .used_percent,
            1.0
        );
    }

    #[test]
    fn a_sign_in_names_the_windows_no_new_thread_reaches() {
        let ledger = Ledger {
            threads: vec![thread("a")],
            readings: vec![
                reading(0, 0, true, (10.0, 300), (40.0, 10_000)),
                // The limits reset early, and a's thread carries on.
                reading(100, 0, false, (0.0, 400), (0.0, 20_000)),
            ],
            ..Ledger::default()
        };
        let signin = |account: &str, from, to| SignIn {
            account: account.into(),
            from: at(from),
            to: at(to),
        };
        let owners = |signins: &[SignIn]| {
            let accounts = Accounts::attribute(&ledger, signins);
            (0..2)
                .map(|index| accounts.of_reading(index).map(|id| &*accounts.ids[id]))
                .map(|id| id.map(str::to_owned))
                .collect::<Vec<_>>()
        };
        assert_eq!(owners(&[]), [Some("a".into()), None]);
        // Signed in to b since minute 50: the new windows are b's. Should a
        // new thread's own reading disagree, it wins.
        assert_eq!(
            owners(&[signin("b", -10, 200)]),
            [Some("a".into()), Some("b".into())]
        );
        // A request perhaps sent before the sign-in names no one.
        assert_eq!(owners(&[signin("b", 101, 200)]), [Some("a".into()), None]);
        // Nor does a reading logged after the file was last seen unchanged.
        assert_eq!(owners(&[signin("b", 50, 99)]), [Some("a".into()), None]);
    }

    #[test]
    fn a_new_window_replaces_the_last_even_before_its_reset() {
        // Limits reset early: the next window's reset is later, so it wins.
        let mut ledger = Ledger {
            threads: vec![thread("a")],
            readings: vec![
                reading(0, 0, true, (50.0, 300), (36.0, 10_000)),
                reading(100, 0, false, (3.0, 400), (2.0, 14_000)),
            ],
            ..Ledger::default()
        };
        let accounts = Accounts::attribute(&ledger, &[]);
        assert_eq!(
            accounts.of_reading(1),
            None,
            "nothing links the new window to a"
        );
        // Linked by a new thread's first reading, the new window becomes a's latest.
        ledger.threads.push(thread("a"));
        ledger
            .readings
            .push(reading(101, 1, true, (4.0, 400), (2.0, 14_000)));
        let accounts = Accounts::attribute(&ledger, &[]);
        let mut a = accounts.tracker(&ledger, 0);
        a.advance_to(at(200));
        assert_eq!(
            a.windows[LONG].as_ref().unwrap().at(at(200)).used_percent,
            2.0
        );
    }

    #[test]
    fn a_window_the_plan_drops_is_gone() {
        // Plus's limits from July to August 2026: a weekly window only, logged as `primary`.
        let weekly_only = |minute, used, reset| {
            let full = reading(minute, 0, false, (0.0, 0), (used, reset));
            let limits = Limits {
                primary: full.limits.secondary.clone(),
                secondary: None,
                ..(*full.limits).clone()
            };
            Reading {
                limits: Arc::new(limits),
                ..full
            }
        };
        let shown = |ledger: &Ledger, minute| {
            let accounts = Accounts::attribute(ledger, &[]);
            let mut tracker = accounts.tracker(ledger, 0);
            tracker.advance_to(at(minute));
            [SHORT, LONG].map(|kind| {
                let window = tracker.windows[kind].as_ref()?;
                Some(window.at(at(minute)).used_percent)
            })
        };
        let mut ledger = Ledger {
            threads: vec![thread("a"), thread("a")],
            readings: vec![
                reading(0, 0, true, (30.0, 300), (40.0, 10_000)),
                // The same week, so the same account's.
                weekly_only(100, 41.0, 10_000),
            ],
            ..Ledger::default()
        };
        assert_eq!(shown(&ledger, 50), [Some(30.0), Some(40.0)]);
        assert_eq!(
            shown(&ledger, 110),
            [None, Some(41.0)],
            "no 5-hour window once the plan drops it, not an idle one"
        );

        // The 5-hour window is back, with a new week; a reading of the old
        // week logged late drops nothing.
        ledger
            .readings
            .push(reading(200, 1, true, (5.0, 500), (1.0, 20_000)));
        ledger.readings.push(weekly_only(210, 42.0, 10_000));
        assert_eq!(shown(&ledger, 220), [Some(5.0), Some(1.0)]);
    }

    /// Account 0's 5-hour and weekly use at `minute`.
    fn used(ledger: &Ledger, minute: i64) -> [f64; 2] {
        let accounts = Accounts::attribute(ledger, &[]);
        let mut tracker = accounts.tracker(ledger, 0);
        tracker.advance_to(at(minute));
        [SHORT, LONG].map(|kind| {
            let window = tracker.windows[kind].as_ref().unwrap();
            window.at(at(minute)).used_percent
        })
    }

    #[test]
    fn a_window_adds_the_responses_its_highest_reading_leaves_out() {
        // Each reading leaves out the response logged with it: the first shows 0%.
        let mut ledger = Ledger {
            threads: vec![thread("a"), thread("a")],
            readings: vec![
                reading(0, 0, true, (0.0, 300), (40.0, 10_000)),
                reading(10, 0, false, (10.0, 300), (41.0, 10_000)),
                reading(20, 0, false, (20.0, 300), (42.0, 10_000)),
            ],
            responses: vec![
                response(0, 0, 0, Some(1.0)),
                response(10, 0, 1, Some(1.0)),
                response(20, 0, 2, Some(0.5)),
            ],
            ..Ledger::default()
        };
        // The first two responses' $2 took 20 points (10 a dollar) and 2 of the week.
        let [five, week] = used(&ledger, 30);
        assert!(close(five, 25.0) && close(week, 42.5), "{five} {week}");

        // Another thread's reading, logged late for a request sent before the
        // third response was charged: lower, so stale. Its own response is not
        // in any reading either.
        ledger.readings.push(Reading {
            since: at(24),
            ..reading(25, 1, false, (18.0, 300), (41.0, 10_000))
        });
        ledger.responses.push(response(24, 1, 3, Some(0.3)));
        let [five, week] = used(&ledger, 30);
        assert!(close(five, 28.0) && close(week, 42.8), "{five} {week}");

        // A new 5-hour window: its first reading shows 0%, its first response not.
        ledger
            .readings
            .push(reading(400, 0, false, (0.0, 700), (43.0, 10_000)));
        ledger.responses.push(response(400, 0, 4, Some(1.0)));
        assert!(close(used(&ledger, 410)[SHORT], 10.0));
        // Up to the limit, and nothing once the window reset.
        ledger.responses[4].cost = Some(CodexCostSplit {
            input: 20.0,
            ..CodexCostSplit::default()
        });
        assert_eq!(used(&ledger, 410)[SHORT], 100.0);
        assert_eq!(used(&ledger, 701)[SHORT], 0.0);
    }

    #[test]
    fn a_newer_reading_at_the_same_use_has_seen_the_responses_before_it() {
        let ledger = Ledger {
            threads: vec![thread("a")],
            readings: vec![
                reading(0, 0, true, (0.0, 300), (1.0, 10_000)),
                reading(10, 0, false, (10.0, 300), (2.0, 10_000)),
                // The second response took under a point.
                reading(20, 0, false, (10.0, 300), (2.0, 10_000)),
            ],
            responses: vec![
                response(0, 0, 0, Some(1.0)),
                response(10, 0, 1, Some(1.0)),
                response(20, 0, 2, Some(1.0)),
            ],
            ..Ledger::default()
        };
        // 10 points for $2: only the third response is left to add, at 5 a dollar.
        assert!(close(used(&ledger, 30)[SHORT], 15.0));
    }

    #[test]
    fn rates_pool_each_plans_windows_that_tell() {
        let plan = |reading: Reading, plan: &str| Reading {
            limits: Arc::new(Limits {
                plan: Some(plan.into()),
                ..(*reading.limits).clone()
            }),
            ..reading
        };
        let ledger = Ledger {
            threads: vec![thread("a")],
            readings: vec![
                // Gains 30 points for $3.
                reading(0, 0, true, (0.0, 300), (1.0, 10_000)),
                reading(10, 0, false, (30.0, 300), (2.0, 10_000)),
                // Reaches the limit: tells nothing of a rate.
                reading(400, 0, false, (0.0, 700), (3.0, 10_000)),
                reading(410, 0, false, (100.0, 700), (8.0, 10_000)),
                // Serves a model without a price.
                reading(800, 0, false, (0.0, 1100), (9.0, 10_000)),
                reading(810, 0, false, (50.0, 1100), (10.0, 10_000)),
                // Another plan's: 2 points for $2.
                plan(reading(1200, 0, false, (0.0, 1500), (0.0, 20_000)), "pro"),
                plan(reading(1210, 0, false, (2.0, 1500), (1.0, 20_000)), "pro"),
            ],
            responses: vec![
                response(0, 0, 0, Some(3.0)),
                response(400, 0, 2, Some(1.0)),
                response(800, 0, 4, None),
                response(1200, 0, 6, Some(2.0)),
            ],
            ..Ledger::default()
        };
        let accounts = Accounts::attribute(&ledger, &[]);
        assert_eq!(accounts.rate(Some("plus"), SHORT), Some(10.0));
        assert_eq!(accounts.rate(Some("pro"), SHORT), Some(1.0));
        assert_eq!(accounts.rate(None, SHORT), None);
        // The plus week served the model without a price too; the pro one gained 1 point for $2.
        assert_eq!(accounts.rate(Some("plus"), LONG), None);
        assert_eq!(accounts.rate(Some("pro"), LONG), Some(0.5));
    }
}
