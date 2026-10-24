//! The snapshot the frontend shows, from the ledger and the clock. Only this
//! part depends on the time, so it runs on every tick; the ledger, with every
//! response's cost, is rebuilt only when a rollout or a price changes.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Days, NaiveDate, TimeDelta, TimeZone, Timelike, Utc};

use super::accounts::{Accounts, LONG, SHORT, Tracker, plan_limits};
use super::ledger::{Ledger, Response};
use super::paths::CodexHome;
use super::signin::SignIns;
use super::types::{
    CodexAccount, CodexContextStats, CodexCurrent, CodexData, CodexDay, CodexEffortRow,
    CodexHistory, CodexModelRow, CodexPoint, CodexSlice, CodexSpanCosts, CodexThread, CodexToday,
    CodexTotal, CodexUsage, TrendPoint,
};
use crate::usage::day_start;

/// Days in the daily series, today included.
const DAILY_DAYS: u64 = 31;
/// The span of the effort, model, context and thread views.
const RECENT_DAYS: i64 = 7;
/// The span of an account's longest sum.
const ACCOUNT_DAYS: i64 = 30;
/// The overview's spans besides today and all time, in local days.
const WEEK_DAYS: i64 = 7;
const MONTH_DAYS: i64 = 30;
/// Context use, as a share of the window, that counts as near full.
const NEAR_FULL: f64 = 0.8;
/// Effort levels in the order they are listed, strongest first. Others follow
/// by name, unknown last.
const EFFORT_ORDER: [&str; 8] = [
    "ultra", "max", "xhigh", "high", "medium", "low", "minimal", "none",
];

/// Thread names from `session_index.jsonl`.
pub type Titles = HashMap<String, String>;

pub fn report<Tz: TimeZone>(
    ledger: &Ledger,
    titles: &Titles,
    home: &CodexHome,
    signins: &SignIns,
    now: DateTime<Utc>,
    tz: &Tz,
) -> CodexData {
    let today = now.with_timezone(tz).date_naive();
    let since = now - TimeDelta::days(RECENT_DAYS);
    let accounts = Accounts::attribute(ledger, signins.spans());

    let grid = trend_grid(now);
    // Each account, and the weekly window it is in.
    let (mut account_rows, weeks): (Vec<CodexAccount>, Vec<Option<usize>>) =
        (0..accounts.ids.len())
            .map(|index| {
                let email = signins.email(&accounts.ids[index]);
                account(ledger, &accounts, index, email, &grid, now)
            })
            .unzip();
    let month = now - TimeDelta::days(ACCOUNT_DAYS);

    let (hourly, daily, series_models) = series(ledger, &accounts, now, today, tz);
    let mut today_usage = CodexUsage::default();
    let mut today_threads = HashSet::new();
    let mut today_subagents = HashSet::new();
    let mut span_costs = CodexSpanCosts::default();
    for response in &ledger.responses {
        let usage = response.usage();
        // Local days back from today; negative for a response logged ahead of the clock.
        let back = (today - response.at.with_timezone(tz).date_naive()).num_days();
        let is_today = back == 0;
        if let Some(cost) = response.cost {
            span_costs.total += cost;
            for (days, span) in [
                (1, &mut span_costs.today),
                (WEEK_DAYS, &mut span_costs.last_7_days),
                (MONTH_DAYS, &mut span_costs.last_30_days),
            ] {
                if (0..days).contains(&back) {
                    *span += cost;
                }
            }
        }
        if let Some(account) = accounts.of_response(response) {
            let row = &mut account_rows[account];
            if is_today {
                row.today += usage;
            }
            // By the window the response was served in, not by its time:
            // the reset is logged with a few seconds' jitter, which can put a
            // window's first response before `resets_at − window_minutes`.
            if let Some(cycle) = &mut row.cycle
                && weeks[account].is_some()
                && accounts.window_of(response, LONG) == weeks[account]
            {
                *cycle += usage;
            }
            if response.at > month {
                row.last_30_days += usage;
            }
        }
        if !is_today {
            continue;
        }
        today_usage += usage;
        let thread = &ledger.threads[response.thread];
        today_threads.insert(thread.root);
        if thread.root != response.thread {
            today_subagents.insert(response.thread);
        }
    }
    let today_turns = ledger
        .turns
        .iter()
        .filter(|turn| turn.at.with_timezone(tz).date_naive() == today)
        .count() as u64;
    // Signed in now: the account behind the latest reading, or the one
    // signed in since, as last seen, until a reading says more.
    let latest = ledger
        .readings
        .iter()
        .enumerate()
        .filter(|(_, reading)| plan_limits(&reading.limits) && reading.at <= now)
        .max_by_key(|(index, reading)| (reading.at, *index))
        .map(|(index, _)| index);
    let switched = signins.current().zip(latest).and_then(|(signin, latest)| {
        let owner = accounts.of_reading(latest)?;
        (accounts.ids[owner] != signin.account && ledger.readings[latest].at < signin.from)
            .then(|| accounts.ids.iter().position(|id| *id == signin.account))
    });
    let current = match switched {
        Some(Some(account)) => Some(current_of(
            accounts.tracker(ledger, account),
            Some(account),
            now,
        )),
        Some(None) => Some(CodexCurrent {
            account: None,
            plan: None,
            five_hour: None,
            weekly: None,
        }),
        None => latest.map(|reading| {
            current_of(
                accounts.tracker_with(ledger, reading),
                accounts.of_reading(reading),
                now,
            )
        }),
    };
    let active_account = current.as_ref().and_then(|current| current.account);
    // Each thread's account: the one its latest response was served under.
    let mut served: Vec<Option<(DateTime<Utc>, usize)>> = vec![None; ledger.threads.len()];
    for response in &ledger.responses {
        if let Some(account) = accounts.of_response(response) {
            let slot = &mut served[response.thread];
            if slot.is_none_or(|(at, _)| response.at >= at) {
                *slot = Some((response.at, account));
            }
        }
    }
    let account_of = |thread: usize| served[thread].map(|(_, account)| account);

    let recent: Vec<&Response> = ledger
        .responses
        .iter()
        .filter(|response| response.at > since)
        .collect();
    CodexData {
        snapshot_id: 0,
        generated_at: now,
        home: home.display(),
        file_count: 0,
        custom_prices: ledger.custom_prices,
        pricing_error: None,
        read_errors: Vec::new(),
        accounts: account_rows,
        active_account,
        current,
        today: CodexToday {
            usage: today_usage,
            turns: today_turns,
            threads: today_threads.len() as u64,
            subagents: today_subagents.len() as u64,
        },
        hourly,
        daily,
        series_models,
        efforts: efforts(ledger, &recent, since),
        models: models(ledger, &recent, since),
        context: context(ledger, &recent, since),
        threads: threads(ledger, titles, home, &account_of, since),
        total: CodexTotal {
            usage: ledger.total,
            turns: ledger.turns.len() as u64,
        },
        span_costs,
    }
}

/// `account`'s windows and plan as of `now`, from its readings.
fn current_of(
    mut tracker: Tracker<'_>,
    account: Option<usize>,
    now: DateTime<Utc>,
) -> CodexCurrent {
    tracker.advance_to(now);
    CodexCurrent {
        account,
        plan: tracker.plan.map(str::to_owned),
        five_hour: tracker.windows[SHORT].as_ref().map(|window| window.at(now)),
        weekly: tracker.windows[LONG].as_ref().map(|window| window.at(now)),
    }
}

/// Every local day from the first response's through today, and all time by
/// model and by effort. A response logged ahead of the clock counts in the
/// total but on no day, as in `series`.
pub fn history<Tz: TimeZone>(ledger: &Ledger, now: DateTime<Utc>, tz: &Tz) -> CodexHistory {
    let today = now.with_timezone(tz).date_naive();
    let date_of = |at: DateTime<Utc>| at.with_timezone(tz).date_naive();
    let first = ledger
        .responses
        .iter()
        .map(|response| date_of(response.at))
        .min()
        .map_or(today, |first| first.min(today));
    let mut daily: Vec<CodexDay> = first
        .iter_days()
        .take_while(|date| *date <= today)
        .map(|date| CodexDay {
            date: date.format("%Y-%m-%d").to_string(),
            usage: CodexUsage::default(),
            turns: 0,
        })
        .collect();
    let day = |at: DateTime<Utc>| {
        let date = date_of(at);
        (first..=today)
            .contains(&date)
            .then(|| (date - first).num_days() as usize)
    };
    for response in &ledger.responses {
        if let Some(day) = day(response.at) {
            daily[day].usage += response.usage();
        }
    }
    for turn in &ledger.turns {
        if let Some(day) = day(turn.at) {
            daily[day].turns += 1;
        }
    }
    let all: Vec<&Response> = ledger.responses.iter().collect();
    CodexHistory {
        generated_at: now,
        daily,
        models: models(ledger, &all, DateTime::<Utc>::MIN_UTC),
        efforts: efforts(ledger, &all, DateTime::<Utc>::MIN_UTC),
        total: CodexTotal {
            usage: ledger.total,
            turns: ledger.turns.len() as u64,
        },
    }
}

/// `a3f2…91c0`: enough to tell accounts apart, nothing more.
fn short_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().filter(|c| *c != '-').collect();
    if chars.len() <= 8 {
        return chars.into_iter().collect();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// An account's windows and trend, and the number of its weekly window
/// while that lasts; `report` adds up what the account spent.
fn account(
    ledger: &Ledger,
    accounts: &Accounts,
    index: usize,
    email: Option<&str>,
    grid: &[DateTime<Utc>],
    now: DateTime<Utc>,
) -> (CodexAccount, Option<usize>) {
    let mut tracker = accounts.tracker(ledger, index);
    // The grid ends at `now`, so the tracker ends there too.
    let week_trend = grid
        .iter()
        .map(|&t| {
            tracker.advance_to(t);
            TrendPoint {
                t: t.timestamp_millis(),
                percent: tracker.windows[LONG]
                    .as_ref()
                    .map(|window| window.at(t).used_percent),
            }
        })
        .collect();
    tracker.advance_to(now);
    let week = tracker.windows[LONG].as_ref();
    let weekly = week.map(|window| window.at(now));
    let row = CodexAccount {
        short_id: short_id(&accounts.ids[index]),
        email: email.map(str::to_owned),
        plan: tracker.plan.map(str::to_owned),
        five_hour: tracker.windows[SHORT].as_ref().map(|window| window.at(now)),
        cycle: weekly.as_ref().map(|_| CodexUsage::default()),
        weekly,
        last_seen: ledger
            .responses
            .iter()
            .filter(|response| accounts.of_response(response) == Some(index))
            .map(|response| response.at)
            .max(),
        today: CodexUsage::default(),
        last_30_days: CodexUsage::default(),
        week_trend,
    };
    (row, week.and_then(|window| window.current(now)))
}

/// The last 7 days: their start, every hour after it, and now.
fn trend_grid(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let start = now - TimeDelta::days(RECENT_DAYS);
    let hour = start
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(start);
    let mut grid = vec![start];
    let mut t = hour + TimeDelta::hours(1);
    while t < now {
        grid.push(t);
        t += TimeDelta::hours(1);
    }
    grid.push(now);
    grid
}

/// Today's local hours up to the current one and the last 31 local days,
/// each point split by account and by model, and the models of that split.
fn series<Tz: TimeZone>(
    ledger: &Ledger,
    accounts: &Accounts,
    now: DateTime<Utc>,
    today: NaiveDate,
    tz: &Tz,
) -> (Vec<CodexPoint>, Vec<CodexPoint>, Vec<Option<String>>) {
    let hour_now = now.with_timezone(tz).hour();
    let today_start = day_start(today, tz);
    let first_day = today - Days::new(DAILY_DAYS - 1);
    // The day and today's hour `at` falls in, if it is within the series.
    let place = |at: DateTime<Utc>| -> (Option<usize>, Option<usize>) {
        let local = at.with_timezone(tz);
        let date = local.date_naive();
        let day = (first_day..=today)
            .contains(&date)
            .then(|| (date - first_day).num_days() as usize);
        let hour = (date == today)
            .then_some(local.hour())
            .filter(|hour| *hour <= hour_now);
        (day, hour.map(|hour| hour as usize))
    };
    // The days' models, most tokens first.
    let mut model_tokens: HashMap<Option<&str>, u64> = HashMap::new();
    for response in &ledger.responses {
        if place(response.at).0.is_some() {
            *model_tokens.entry(response.model.as_deref()).or_default() += response.tokens.total();
        }
    }
    let mut models: Vec<(Option<&str>, u64)> = model_tokens.into_iter().collect();
    models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let model_index: HashMap<Option<&str>, usize> = models
        .iter()
        .enumerate()
        .map(|(index, (model, _))| (*model, index))
        .collect();

    let point = |key: String, start: DateTime<Utc>, current: bool| CodexPoint {
        key,
        start,
        current,
        usage: CodexUsage::default(),
        turns: 0,
        by_account: vec![CodexSlice::default(); accounts.ids.len() + 1],
        by_model: vec![CodexSlice::default(); models.len()],
    };
    let mut hourly: Vec<CodexPoint> = (0..=hour_now)
        .map(|hour| {
            let start = today
                .and_hms_opt(hour, 0, 0)
                .and_then(|local| tz.from_local_datetime(&local).earliest())
                .map_or(today_start + TimeDelta::hours(i64::from(hour)), |start| {
                    start.with_timezone(&Utc)
                });
            point(format!("h{hour}"), start, hour == hour_now)
        })
        .collect();
    let mut daily: Vec<CodexPoint> = first_day
        .iter_days()
        .take(DAILY_DAYS as usize)
        .map(|date| {
            let key = date.format("%Y-%m-%d").to_string();
            point(key, day_start(date, tz), date == today)
        })
        .collect();
    for response in &ledger.responses {
        // Today's hours lie within the days.
        let (Some(day), hour) = place(response.at) else {
            continue;
        };
        let usage = response.usage();
        // Responses no account could be traced to come after the accounts.
        let account = accounts.of_response(response).unwrap_or(accounts.ids.len());
        let model = model_index[&response.model.as_deref()];
        let hour = hour.map(|hour| &mut hourly[hour]);
        for point in std::iter::once(&mut daily[day]).chain(hour) {
            point.usage += usage;
            point.by_account[account] += usage;
            point.by_model[model] += usage;
        }
    }
    for turn in &ledger.turns {
        let (day, hour) = place(turn.at);
        if let Some(day) = day {
            daily[day].turns += 1;
        }
        if let Some(hour) = hour {
            hourly[hour].turns += 1;
        }
    }
    let models = models
        .into_iter()
        .map(|(model, _)| model.map(str::to_owned))
        .collect();
    (hourly, daily, models)
}

fn effort_rank(effort: Option<&str>) -> (usize, String) {
    match effort {
        Some(effort) => (
            EFFORT_ORDER
                .iter()
                .position(|known| *known == effort)
                .unwrap_or(EFFORT_ORDER.len()),
            effort.to_owned(),
        ),
        None => (EFFORT_ORDER.len() + 1, String::new()),
    }
}

fn efforts(ledger: &Ledger, recent: &[&Response], since: DateTime<Utc>) -> Vec<CodexEffortRow> {
    let mut rows: HashMap<Option<Arc<str>>, (u64, CodexUsage)> = HashMap::new();
    for response in recent {
        rows.entry(response.effort.clone()).or_default().1 += response.usage();
    }
    for turn in ledger.turns.iter().filter(|turn| turn.at > since) {
        rows.entry(turn.effort.clone()).or_default().0 += 1;
    }
    let mut rows: Vec<CodexEffortRow> = rows
        .into_iter()
        .map(|(effort, (turns, usage))| CodexEffortRow {
            effort: effort.map(|effort| effort.to_string()),
            turns,
            usage,
        })
        .collect();
    rows.sort_by_key(|row| effort_rank(row.effort.as_deref()));
    rows
}

fn models(ledger: &Ledger, recent: &[&Response], since: DateTime<Utc>) -> Vec<CodexModelRow> {
    #[derive(Default)]
    struct Row {
        turns: u64,
        usage: CodexUsage,
        window: Option<(DateTime<Utc>, u64)>,
    }
    let mut rows: HashMap<Option<Arc<str>>, Row> = HashMap::new();
    for response in recent {
        let row = rows.entry(response.model.clone()).or_default();
        row.usage += response.usage();
        if let Some(window) = response.window
            && row.window.is_none_or(|(at, _)| response.at >= at)
        {
            row.window = Some((response.at, window));
        }
    }
    for turn in ledger.turns.iter().filter(|turn| turn.at > since) {
        rows.entry(turn.model.clone()).or_default().turns += 1;
    }
    let mut rows: Vec<CodexModelRow> = rows
        .into_iter()
        .map(|(model, row)| CodexModelRow {
            model: model.map(|model| model.to_string()),
            context_window: row.window.map(|(_, window)| window),
            turns: row.turns,
            usage: row.usage,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.usage
            .tokens
            .total()
            .cmp(&a.usage.tokens.total())
            .then_with(|| a.model.cmp(&b.model))
    });
    rows
}

fn context(ledger: &Ledger, recent: &[&Response], since: DateTime<Utc>) -> CodexContextStats {
    let mut near_full = HashSet::new();
    let mut subagents = HashSet::new();
    let mut spawning = HashSet::new();
    let mut stats = CodexContextStats::default();
    for response in recent {
        let usage = response.usage();
        stats.total += usage;
        if let Some(window) = response.window
            && response.tokens.total() as f64 >= NEAR_FULL * window as f64
        {
            near_full.insert(response.thread);
        }
        let root = ledger.threads[response.thread].root;
        if root != response.thread {
            stats.subagent += usage;
            subagents.insert(response.thread);
            spawning.insert(root);
        }
    }
    stats.near_full = near_full.len() as u64;
    stats.subagents = subagents.len() as u64;
    stats.spawning_threads = spawning.len() as u64;
    stats.compactions = ledger
        .compactions
        .iter()
        .filter(|compaction| compaction.at > since)
        .count() as u64;
    stats
}

/// Top-level threads active in the last 7 days, each with its subagents folded in.
fn threads(
    ledger: &Ledger,
    titles: &Titles,
    home: &CodexHome,
    account_of: &impl Fn(usize) -> Option<usize>,
    since: DateTime<Utc>,
) -> Vec<CodexThread> {
    #[derive(Default)]
    struct Folded {
        usage: CodexUsage,
        turns: u64,
        compactions: u64,
        subagents: u64,
        subagent_tokens: u64,
        last_activity: Option<DateTime<Utc>>,
    }
    let mut folded: BTreeMap<usize, Folded> = BTreeMap::new();
    for (index, thread) in ledger.threads.iter().enumerate() {
        let row = folded.entry(thread.root).or_default();
        row.usage += thread.usage;
        row.turns += thread.turns;
        row.compactions += thread.compactions;
        if thread.root != index {
            row.subagents += 1;
            row.subagent_tokens += thread.usage.tokens.total();
        }
        row.last_activity = row.last_activity.max(thread.last_activity);
    }
    let mut rows: Vec<CodexThread> = folded
        .into_iter()
        .filter_map(|(index, row)| {
            let last_activity = row.last_activity.filter(|at| *at > since)?;
            let thread = &ledger.threads[index];
            let title = titles
                .get(&*thread.id)
                .cloned()
                .or_else(|| thread.subagent.then(|| thread.nickname.clone()).flatten())
                .or_else(|| thread.first_prompt.clone())
                .or_else(|| thread.nickname.clone());
            let total = row.usage.tokens.total();
            Some(CodexThread {
                id: thread.id.to_string(),
                title,
                path: thread.cwd.as_deref().map(|cwd| home.display_path(cwd)),
                model: thread.model.as_deref().map(str::to_owned),
                effort: thread.effort.as_deref().map(str::to_owned),
                account: account_of(index),
                turns: row.turns,
                usage: row.usage,
                context_peak: thread.peak,
                context_window: thread.peak_window,
                compactions: row.compactions,
                subagents: row.subagents,
                subagent_share: if total == 0 {
                    0.0
                } else {
                    row.subagent_tokens as f64 / total as f64
                },
                last_activity,
            })
        })
        .collect();
    rows.sort_by_key(|row| {
        (
            Reverse(row.usage.tokens.total()),
            Reverse(row.last_activity),
            row.id.clone(),
        )
    });
    rows
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;
    use crate::codex::parser::FileLog;
    use crate::codex::types::CodexCostSplit;
    use crate::usage::Pricing;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn log(lines: &[String]) -> FileLog {
        let mut log = FileLog::default();
        for line in lines {
            log.ingest(line.as_bytes());
        }
        log
    }

    fn meta(at: &str, id: &str, extra: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{at}","cwd":"/work/app"{extra}}}}}"#
        )
    }

    fn turn(at: &str, id: &str, model: &str, effort: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"turn_context","payload":{{"turn_id":"{id}","model":"{model}","effort":"{effort}"}}}}"#
        )
    }

    fn count(
        at: &str,
        total: (u64, u64, u64, u64),
        last: (u64, u64, u64, u64),
        limits: &str,
    ) -> String {
        let usage = |(input, cached, output, reasoning): (u64, u64, u64, u64)| {
            format!(
                r#"{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":{reasoning}}}"#
            )
        };
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{},"last_token_usage":{},"model_context_window":1000}}{limits}}}}}"#,
            usage(total),
            usage(last)
        )
    }

    fn limits(five_hour: f64, five_reset: i64, week: f64, week_reset: i64) -> String {
        format!(
            r#","rate_limits":{{"limit_id":"codex","primary":{{"used_percent":{five_hour},"window_minutes":300,"resets_at":{five_reset}}},"secondary":{{"used_percent":{week},"window_minutes":10080,"resets_at":{week_reset}}},"plan_type":"plus"}}"#
        )
    }

    #[test]
    fn reports_windows_series_threads_and_subagents() {
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = at("2026-10-19T16:30:00Z"); // 12:30 local
        let account = r#","creator_account_id":"59d12126-8694-4542-868a-210300328189""#;
        let (five_reset, week_reset) = (now.timestamp() + 600, now.timestamp() + 86_400);
        let root = log(&[
            meta("2026-10-19T13:00:00Z", "root", account),
            turn("2026-10-19T13:00:01Z", "t1", "gpt-6-astra", "high"),
            count("2026-10-19T13:00:02Z", (100, 50, 10, 4), (100, 50, 10, 4), &limits(10.0, five_reset, 40.0, week_reset)),
            turn("2026-10-19T15:10:00Z", "t2", "gpt-6.1-sol", "ultra"),
            count(
                "2026-10-19T15:10:01Z",
                (1000, 650, 900, 300),
                (900, 600, 890, 296),
                // The same windows, the reset logged a few seconds off.
                &limits(12.0, five_reset + 4, 41.0, week_reset - 3),
            ),
            r#"{"timestamp":"2026-10-19T15:20:00Z","type":"compacted","payload":{"compaction_response_id":null}}"#.into(),
        ]);
        let spawn = r#","parent_thread_id":"root","source":{"subagent":{"thread_spawn":{"parent_thread_id":"root"}}},"agent_nickname":"Mill""#;
        let sub = log(&[
            meta("2026-10-19T15:11:00Z", "sub", &format!("{spawn}{account}")),
            turn("2026-10-19T15:11:01Z", "s1", "gpt-6.1-sol", "ultra"),
            count(
                "2026-10-19T15:11:02Z",
                (200, 100, 20, 5),
                (200, 100, 20, 5),
                &limits(11.0, five_reset, 40.0, week_reset),
            ),
        ]);
        // No limits logged (another provider, or Codex before 0.158): no account.
        let old = log(&[
            meta("2026-10-01T10:00:00Z", "old", ""),
            turn("2026-10-01T10:00:01Z", "o1", "gpt-5.5", "xhigh"),
            count("2026-10-01T10:00:02Z", (50, 0, 5, 0), (50, 0, 5, 0), ""),
        ]);
        let ledger = Ledger::build(
            [("r.jsonl", &root), ("s.jsonl", &sub), ("o.jsonl", &old)],
            &Pricing::default(),
        );
        let mut titles = Titles::new();
        titles.insert("root".into(), "Wire Codex rollouts".into());
        let home = CodexHome::from(std::path::PathBuf::from("/codex"));
        let data = report(&ledger, &titles, &home, &SignIns::default(), now, &tz);

        assert_eq!(data.total.usage.tokens.total(), 1900 + 220 + 55);
        assert_eq!(data.total.usage.unpriced, 0);
        assert_eq!(data.today.usage.tokens.total(), 1900 + 220);
        assert_eq!(
            (data.today.turns, data.today.threads, data.today.subagents),
            (3, 1, 1)
        );
        // 09:00 and 11:00 local.
        assert_eq!(data.hourly.len(), 13);
        assert_eq!(data.hourly[9].usage.tokens.total(), 110);
        assert_eq!(data.hourly[11].usage.tokens.total(), 1790 + 220);
        assert_eq!(data.hourly[11].turns, 2);
        assert!(data.hourly[12].current);
        assert_eq!(data.daily.len(), 31);
        assert_eq!(data.daily[30].key, "2026-10-19");
        assert_eq!(data.daily[30].start, at("2026-10-19T04:00:00Z"));
        assert_eq!(data.daily[12].key, "2026-10-01");
        assert_eq!(data.daily[12].usage.tokens.total(), 55);

        let efforts: Vec<_> = data
            .efforts
            .iter()
            .map(|row| (row.effort.as_deref(), row.turns))
            .collect();
        assert_eq!(
            efforts,
            [(Some("ultra"), 2), (Some("high"), 1)],
            "the old turn is past 7 days"
        );
        assert_eq!(data.models[0].model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(data.models[0].usage.tokens.total(), 1790 + 220);
        assert_eq!(data.models[0].context_window, Some(1000));

        assert_eq!(data.context.near_full, 1, "1790 of a 1000-token window");
        assert_eq!(data.context.compactions, 1);
        assert_eq!(
            (data.context.subagents, data.context.spawning_threads),
            (1, 1)
        );
        assert_eq!(data.context.subagent.tokens.total(), 220);

        assert_eq!(data.threads.len(), 1);
        let row = &data.threads[0];
        assert_eq!(row.title.as_deref(), Some("Wire Codex rollouts"));
        assert_eq!(row.usage.tokens.total(), 1900 + 220);
        assert_eq!((row.turns, row.subagents, row.compactions), (3, 1, 1));
        assert!((row.subagent_share - 220.0 / 2120.0).abs() < 1e-12);
        assert_eq!((row.context_peak, row.context_window), (1790, Some(1000)));
        assert_eq!(row.model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(row.account, Some(0));

        assert_eq!(data.accounts.len(), 1);
        let account = &data.accounts[0];
        assert_eq!(account.short_id, "59d1…8189");
        assert_eq!(account.plan.as_deref(), Some("plus"));
        // The highest reading of each window (the subagent's later 11% and 40%
        // are stale), plus the responses no reading shows yet, the second
        // turn's $0.00956 and the subagent's $0.00041, at the points per
        // dollar each window gained for the first turn's $0.00105.
        let unshown = (0.00956 + 0.00041) / 0.00105;
        let five = account.five_hour.as_ref().unwrap().used_percent;
        let week = account.weekly.as_ref().unwrap().used_percent;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(
            close(five, 12.0 + 2.0 * unshown) && close(week, 41.0 + unshown),
            "{five} {week}"
        );
        assert_eq!(account.today.tokens.total(), 2120);
        assert_eq!(account.last_seen, Some(at("2026-10-19T15:11:02Z")));
        assert_eq!(data.active_account, Some(0));
        let current = data.current.as_ref().unwrap();
        assert_eq!(current.account, Some(0));
        assert_eq!(current.five_hour.as_ref().unwrap().used_percent, five);
        let trend = &account.week_trend;
        assert_eq!(trend.first().unwrap().percent, None);
        assert_eq!(trend.last().unwrap().percent, Some(week));
        assert_eq!(trend.last().unwrap().t, now.timestamp_millis());

        // Eleven minutes on, the 5-hour window has reset.
        let later = report(
            &ledger,
            &titles,
            &home,
            &SignIns::default(),
            now + TimeDelta::minutes(11),
            &tz,
        );
        let five = later.accounts[0].five_hour.as_ref().unwrap();
        assert_eq!((five.used_percent, five.resets_at), (0.0, None));
        assert_eq!(five.observed_at, at("2026-10-19T15:11:02Z"));
        assert_eq!(
            later.accounts[0].weekly.as_ref().unwrap().used_percent,
            week
        );
    }

    #[test]
    fn prices_every_view_and_splits_the_series_by_account_and_model() {
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = at("2026-10-19T16:30:00Z"); // 12:30 local
        let a = r#","creator_account_id":"aaaaaaaa-0000-0000-0000-00000000000a""#;
        let b = r#","creator_account_id":"bbbbbbbb-0000-0000-0000-00000000000b""#;
        let reset = |text: &str| at(text).timestamp();
        // a's weekly window opened with a2's request on the 15th at 12:00:00
        // (the one before reset on the 12th); its later readings log the reset
        // 3 s later, past a2's response at 12:00:02.
        let a_week = reset("2026-10-22T12:00:00Z");
        let a_week_later = reset("2026-10-22T12:00:03Z");
        let a_five = now.timestamp() + 600;
        let b_week = (now + TimeDelta::days(5)).timestamp();
        // One response per thread: (input, cached, output, reasoning), the
        // same as total and as last.
        let thread =
            |start: &str, id: &str, extra: &str, model: &str, effort: &str, used, limits: &str| {
                let time = |second: u32| start.replace(":00Z", &format!(":{second:02}Z"));
                log(&[
                    meta(start, id, extra),
                    turn(&time(1), &format!("{id}-turn"), model, effort),
                    count(&time(2), used, used, limits),
                ])
            };
        let spawn = r#","parent_thread_id":"a1","source":{"subagent":{"thread_spawn":{"parent_thread_id":"a1"}}}"#;
        let logs = [
            // $0.03: 10K × $2 + 1K × $10.
            thread(
                "2026-10-10T12:00:00Z",
                "a0",
                a,
                "gpt-6.1-sol",
                "medium",
                (10_000, 0, 1_000, 0),
                &limits(
                    3.0,
                    reset("2026-10-10T16:00:00Z"),
                    20.0,
                    reset("2026-10-12T00:00:00Z"),
                ),
            ),
            // $0.19: 10K × $10 + 40K × $1 + 1K × $50.
            thread(
                "2026-10-15T12:00:00Z",
                "a2",
                a,
                "gpt-6-astra",
                "high",
                (50_000, 40_000, 1_000, 0),
                &limits(5.0, reset("2026-10-15T16:00:00Z"), 10.0, a_week),
            ),
            // $0.56: 40K × $10 + 60K × $1 + 2K × $50.
            thread(
                "2026-10-19T13:00:00Z",
                "a1",
                a,
                "gpt-6-astra",
                "high",
                (100_000, 60_000, 2_000, 500),
                &limits(10.0, a_five, 40.0, a_week_later),
            ),
            // $0.135: 10K × $10 + 10K × $1 + 500 × $50.
            thread(
                "2026-10-19T13:10:00Z",
                "a1s",
                &format!("{spawn}{a}"),
                "gpt-6-astra",
                "high",
                (20_000, 10_000, 500, 100),
                &limits(11.0, a_five, 40.0, a_week_later),
            ),
            // No price.
            thread(
                "2026-10-19T14:00:00Z",
                "b1",
                b,
                "gpt-reserve",
                "max",
                (30_000, 0, 500, 0),
                &limits(2.0, now.timestamp() + 7200, 5.0, b_week),
            ),
            // No limits, so no account. $0.0004: 50 × $5 + 5 × $30.
            thread(
                "2026-10-01T10:00:00Z",
                "old",
                "",
                "gpt-5.5",
                "xhigh",
                (50, 0, 5, 0),
                "",
            ),
        ];
        let names = [
            "a0.jsonl",
            "a2.jsonl",
            "a1.jsonl",
            "a1s.jsonl",
            "b1.jsonl",
            "old.jsonl",
        ];
        let ledger = Ledger::build(names.into_iter().zip(&logs), &Pricing::default());
        let home = CodexHome::from(std::path::PathBuf::from("/codex"));
        let data = report(
            &ledger,
            &Titles::new(),
            &home,
            &SignIns::default(),
            now,
            &tz,
        );
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;

        assert_eq!(
            data.total.usage.tokens.total(),
            11_000 + 51_000 + 102_000 + 20_500 + 30_500 + 55
        );
        assert!(close(
            data.total.usage.cost_usd,
            0.03 + 0.19 + 0.56 + 0.135 + 0.0004
        ));
        assert_eq!((data.total.usage.unpriced, data.custom_prices), (1, 0));
        assert_eq!(data.total.turns, 6);
        // Today: a1 with its subagent, and b1.
        assert_eq!(data.today.usage.tokens.total(), 122_500 + 30_500);
        assert!(close(data.today.usage.cost_usd, 0.695));
        assert_eq!(data.today.usage.unpriced, 1);

        // The overview's spans by kind of token: (input, cached, output, reasoning).
        let split = |cost: CodexCostSplit| (cost.input, cost.cached, cost.output, cost.reasoning);
        let costs = data.span_costs;
        let same = |(a, b, c, d): (f64, f64, f64, f64), (e, f, g, h): (f64, f64, f64, f64)| {
            close(a, e) && close(b, f) && close(c, g) && close(d, h)
        };
        // a1: 40K × $10 + 60K × $1, 2K × $50 of which 500 reasoning; a1s:
        // 10K × $10 + 10K × $1, 500 × $50 of which 100 reasoning. b1 is unpriced.
        assert!(
            same(split(costs.today), (0.57, 0.07, 0.125, 0.03)),
            "{costs:?}"
        );
        assert!(close(costs.today.total(), data.today.usage.cost_usd));
        // And a2 on the 15th: 10K × $10 + 40K × $1, 1K × $50.
        assert!(same(split(costs.last_7_days), (0.71, 0.11, 0.175, 0.03)));
        // And a0 on the 10th (10K × $2, 1K × $10) and the old thread on the 1st.
        let month = (0.730_25, 0.11, 0.185_15, 0.03);
        assert!(same(split(costs.last_30_days), month));
        assert!(same(split(costs.total), month));
        assert!(close(costs.total.total(), data.total.usage.cost_usd));
        let week: f64 = data.daily[24..].iter().map(|day| day.usage.cost_usd).sum();
        assert!(
            close(costs.last_7_days.total(), week),
            "the last 7 points of `daily`"
        );

        // Two accounts and the unattributed; models by their tokens over the days.
        let models: Vec<_> = data.series_models.iter().map(Option::as_deref).collect();
        assert_eq!(
            models,
            [
                Some("gpt-6-astra"),
                Some("gpt-reserve"),
                Some("gpt-6.1-sol"),
                Some("gpt-5.5")
            ]
        );
        let nine = &data.hourly[9];
        assert_eq!(nine.usage.tokens.total(), 122_500);
        assert!(close(nine.usage.cost_usd, 0.695));
        assert_eq!(nine.by_account.len(), 3);
        assert!(nine.by_account[0].0 == 122_500 && close(nine.by_account[0].1, 0.695));
        assert!(nine.by_model[0].0 == 122_500 && close(nine.by_model[0].1, 0.695));
        let ten = &data.hourly[10];
        assert_eq!(ten.usage.unpriced, 1);
        assert_eq!(
            ten.by_account,
            [
                CodexSlice(0, 0.0),
                CodexSlice(30_500, 0.0),
                CodexSlice(0, 0.0)
            ]
        );
        assert_eq!(ten.by_model[1], CodexSlice(30_500, 0.0));
        let today = &data.daily[30];
        assert!(today.by_account[0].0 == 122_500 && close(today.by_account[0].1, 0.695));
        assert_eq!(today.by_account[1], CodexSlice(30_500, 0.0));
        // The 1st's response belongs to no account, and is the only gpt-5.5 one.
        let first = &data.daily[12];
        assert!(first.by_account[2].0 == 55 && close(first.by_account[2].1, 0.0004));
        assert!(first.by_model[3].0 == 55 && close(first.by_model[3].1, 0.0004));
        assert!(
            data.daily[21].by_model[2].0 == 11_000 && close(data.daily[21].by_model[2].1, 0.03)
        );
        for point in data.hourly.iter().chain(&data.daily) {
            for split in [&point.by_account, &point.by_model] {
                let tokens: u64 = split.iter().map(|slice| slice.0).sum();
                assert_eq!(tokens, point.usage.tokens.total(), "{}", point.key);
                assert!(
                    close(
                        split.iter().map(|slice| slice.1).sum(),
                        point.usage.cost_usd
                    ),
                    "{}",
                    point.key
                );
            }
        }

        // The last 7 days leave out a0 and the old thread.
        let efforts: Vec<_> = data
            .efforts
            .iter()
            .map(|row| {
                (
                    row.effort.as_deref(),
                    row.turns,
                    row.usage.tokens.total(),
                    row.usage.unpriced,
                )
            })
            .collect();
        assert_eq!(
            efforts,
            [(Some("max"), 1, 30_500, 1), (Some("high"), 3, 173_500, 0)]
        );
        assert!(close(data.efforts[1].usage.cost_usd, 0.885));
        let models: Vec<_> = data
            .models
            .iter()
            .map(|row| {
                (
                    row.model.as_deref(),
                    row.turns,
                    row.usage.tokens.total(),
                    row.usage.unpriced,
                )
            })
            .collect();
        assert_eq!(
            models,
            [
                (Some("gpt-6-astra"), 3, 173_500, 0),
                (Some("gpt-reserve"), 1, 30_500, 1)
            ]
        );
        assert!(close(data.models[0].usage.cost_usd, 0.885));
        let context = &data.context;
        assert_eq!(
            (context.total.tokens.total(), context.total.unpriced),
            (204_000, 1)
        );
        assert!(close(context.total.cost_usd, 0.885));
        assert_eq!(context.subagent.tokens.total(), 20_500);
        assert!(close(context.subagent.cost_usd, 0.135));
        let threads: Vec<_> = data
            .threads
            .iter()
            .map(|row| {
                (
                    row.id.as_str(),
                    row.usage.tokens.total(),
                    row.usage.unpriced,
                )
            })
            .collect();
        assert_eq!(
            threads,
            [("a1", 122_500, 0), ("a2", 51_000, 0), ("b1", 30_500, 1)]
        );
        assert!(close(data.threads[0].usage.cost_usd, 0.695));

        let [a, b] = &data.accounts[..] else {
            panic!("{:?}", data.accounts);
        };
        // a: today a1 with its subagent. In its weekly window a2 as well,
        // logged a second before `resets_at − 7 days`. In 30 days a0 too.
        assert_eq!(a.today.tokens.total(), 122_500);
        assert!(close(a.today.cost_usd, 0.695));
        let weekly = a.weekly.as_ref().unwrap();
        assert_eq!(weekly.resets_at, Some(at("2026-10-22T12:00:03Z")));
        let cycle = a.cycle.unwrap();
        assert_eq!(cycle.tokens.total(), 173_500);
        assert!(close(cycle.cost_usd, 0.885));
        assert_eq!(a.last_30_days.tokens.total(), 184_500);
        assert!(close(a.last_30_days.cost_usd, 0.915));
        for usage in [b.today, b.cycle.unwrap(), b.last_30_days] {
            assert_eq!(
                (usage.tokens.total(), usage.cost_usd, usage.unpriced),
                (30_500, 0.0, 1)
            );
        }
        // Three days on, a's weekly window has reset and nothing has used the next.
        let later = report(
            &ledger,
            &Titles::new(),
            &home,
            &SignIns::default(),
            now + TimeDelta::days(3),
            &tz,
        );
        assert_eq!(later.accounts[0].cycle, Some(CodexUsage::default()));
        assert_eq!(later.accounts[1].cycle.unwrap().tokens.total(), 30_500);
    }

    #[test]
    fn sign_ins_name_old_threads_windows_and_a_switch_shows_at_once() {
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let (a, b) = (
            "aaaa1111-0000-4000-8000-00000000aaaa",
            "bbbb2222-0000-4000-8000-00000000bbbb",
        );
        let creator = |id: &str| format!(r#","creator_account_id":"{id}""#);
        let started = |at: &str, turn: &str| {
            format!(
                r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"task_started","turn_id":"{turn}"}}}}"#
            )
        };
        let reset = |text: &str| at(text).timestamp();
        let one = (100, 0, 10, 0);
        let ta = log(&[
            meta("2026-10-23T10:00:00Z", "ta", &creator(a)),
            started("2026-10-23T10:00:01Z", "a1"),
            count(
                "2026-10-23T10:00:05Z",
                one,
                one,
                &limits(
                    20.0,
                    reset("2026-10-23T14:00:00Z"),
                    30.0,
                    reset("2026-10-28T10:00:00Z"),
                ),
            ),
        ]);
        let tb = log(&[
            meta("2026-10-23T11:00:00Z", "tb", &creator(b)),
            started("2026-10-23T11:00:01Z", "b1"),
            count(
                "2026-10-23T11:00:05Z",
                one,
                one,
                &limits(
                    5.0,
                    reset("2026-10-23T16:00:00Z"),
                    10.0,
                    reset("2026-10-29T11:00:00Z"),
                ),
            ),
            // Signed in to a at 11:30, whose limits then reset early: b's
            // thread carries on in windows no new thread reaches.
            started("2026-10-23T12:00:00Z", "b2"),
            count(
                "2026-10-23T12:00:05Z",
                (200, 0, 20, 0),
                one,
                &limits(
                    0.0,
                    reset("2026-10-23T17:00:00Z"),
                    0.0,
                    reset("2026-10-30T12:00:00Z"),
                ),
            ),
        ]);
        let ledger = Ledger::build([("a.jsonl", &ta), ("b.jsonl", &tb)], &Pricing::default());
        let home = CodexHome::from(std::path::PathBuf::from("/codex"));
        let signin = |account: &str, from: &str, to: &str| crate::codex::signin::SignIn {
            account: account.into(),
            from: at(from),
            to: at(to),
        };
        let weekly_reset = |data: &CodexData, account: usize| {
            data.accounts[account].weekly.as_ref().unwrap().resets_at
        };

        let now = at("2026-10-23T12:10:00Z");
        let unknown = report(
            &ledger,
            &Titles::new(),
            &home,
            &SignIns::default(),
            now,
            &tz,
        );
        assert_eq!(unknown.current.as_ref().unwrap().account, None);
        assert_eq!(
            weekly_reset(&unknown, 0),
            Some(at("2026-10-28T10:00:00Z")),
            "a as before the reset"
        );

        let signins = SignIns::seen(vec![
            signin(b, "2026-10-23T10:30:00Z", "2026-10-23T11:30:00Z"),
            signin(a, "2026-10-23T11:30:00Z", "2026-10-23T12:10:00Z"),
        ]);
        let known = report(&ledger, &Titles::new(), &home, &signins, now, &tz);
        assert_eq!(known.current.as_ref().unwrap().account, Some(0));
        assert_eq!(
            weekly_reset(&known, 0),
            Some(at("2026-10-30T12:00:00Z")),
            "a since the reset"
        );
        assert_eq!(
            weekly_reset(&known, 1),
            Some(at("2026-10-29T11:00:00Z")),
            "b untouched"
        );

        // Switched to b at 12:20, nothing logged since: b's limits as last seen.
        let later = at("2026-10-23T12:30:00Z");
        let signins = SignIns::seen(vec![
            signin(b, "2026-10-23T10:30:00Z", "2026-10-23T11:30:00Z"),
            signin(a, "2026-10-23T11:30:00Z", "2026-10-23T12:20:00Z"),
            signin(b, "2026-10-23T12:20:00Z", "2026-10-23T12:30:00Z"),
        ]);
        let switched = report(&ledger, &Titles::new(), &home, &signins, later, &tz);
        let current = switched.current.unwrap();
        assert_eq!(current.account, Some(1));
        assert_eq!(
            current.weekly.unwrap().resets_at,
            Some(at("2026-10-29T11:00:00Z"))
        );
    }

    #[test]
    fn a_switch_to_an_unseen_account_clears_the_previous_limits() {
        let now = at("2026-10-23T12:00:00Z");
        let home = CodexHome::from(std::path::PathBuf::from("/codex"));
        let signins = SignIns::seen(vec![crate::codex::signin::SignIn {
            account: "b".into(),
            from: at("2026-10-23T11:00:00Z"),
            to: now,
        }]);
        for creator in [r#","creator_account_id":"a""#, ""] {
            let log = log(&[
                meta("2026-10-23T10:00:00Z", "a", creator),
                count(
                    "2026-10-23T10:00:05Z",
                    (100, 0, 10, 0),
                    (100, 0, 10, 0),
                    &limits(20.0, now.timestamp() + 3600, 30.0, now.timestamp() + 86_400),
                ),
            ]);
            let ledger = Ledger::build([("a.jsonl", &log)], &Pricing::default());
            let data = report(&ledger, &Titles::new(), &home, &signins, now, &Utc);
            let current = data.current.unwrap();
            assert_eq!(current.account, None);
            assert_eq!(data.active_account, None);
            if creator.is_empty() {
                assert!(current.five_hour.is_some() && current.weekly.is_some());
            } else {
                assert_eq!(data.accounts.len(), 1);
                assert!(current.plan.is_none());
                assert!(current.five_hour.is_none() && current.weekly.is_none());
            }
        }
    }

    #[test]
    fn history_covers_every_day_since_the_first_response() {
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = at("2026-10-19T16:30:00Z"); // 12:30 local
        // One response per thread: (input, cached, output, reasoning).
        let thread = |start: &str, id: &str, model: &str, effort: &str, used| {
            let time = |second: u32| start.replace(":00Z", &format!(":{second:02}Z"));
            log(&[
                meta(start, id, ""),
                turn(&time(1), &format!("{id}-turn"), model, effort),
                count(&time(2), used, used, ""),
            ])
        };
        let logs = [
            // Long before the snapshot's 31 days.
            thread(
                "2026-08-01T14:00:00Z",
                "early",
                "gpt-6.1-sol",
                "medium",
                (10_000, 0, 1_000, 0),
            ),
            // 23:30 local on the 18th.
            thread(
                "2026-10-19T03:30:00Z",
                "late",
                "gpt-6-astra",
                "high",
                (50_000, 40_000, 1_000, 200),
            ),
            thread(
                "2026-10-19T13:00:00Z",
                "today",
                "gpt-6-astra",
                "xhigh",
                (100_000, 60_000, 2_000, 500),
            ),
        ];
        let names = ["early.jsonl", "late.jsonl", "today.jsonl"];
        let ledger = Ledger::build(names.into_iter().zip(&logs), &Pricing::default());
        let history = history(&ledger, now, &tz);

        // August 1st through October 19th.
        assert_eq!(history.daily.len(), 31 + 30 + 19);
        let day = |index: usize| {
            let day = &history.daily[index];
            (day.date.as_str(), day.usage.tokens.total(), day.turns)
        };
        assert_eq!(day(0), ("2026-08-01", 11_000, 1));
        assert_eq!(day(1), ("2026-08-02", 0, 0));
        assert_eq!(day(78), ("2026-10-18", 51_000, 1));
        assert_eq!(day(79), ("2026-10-19", 102_000, 1));
        let days: u64 = history
            .daily
            .iter()
            .map(|day| day.usage.tokens.total())
            .sum();
        assert_eq!(days, history.total.usage.tokens.total());
        assert_eq!(history.total.turns, 3);

        // All time, where the snapshot's views cover 7 days.
        let models: Vec<_> = history
            .models
            .iter()
            .map(|row| (row.model.as_deref(), row.turns, row.usage.tokens.total()))
            .collect();
        assert_eq!(
            models,
            [
                (Some("gpt-6-astra"), 2, 153_000),
                (Some("gpt-6.1-sol"), 1, 11_000)
            ]
        );
        let efforts: Vec<_> = history
            .efforts
            .iter()
            .map(|row| (row.effort.as_deref(), row.turns, row.usage.tokens.total()))
            .collect();
        assert_eq!(
            efforts,
            [
                (Some("xhigh"), 1, 102_000),
                (Some("high"), 1, 51_000),
                (Some("medium"), 1, 11_000)
            ]
        );
    }

    #[test]
    fn history_of_no_use_is_today_alone() {
        let tz = FixedOffset::west_opt(4 * 3600).unwrap();
        let now = at("2026-10-19T16:30:00Z");
        let history = history(&Ledger::default(), now, &tz);
        assert_eq!(history.daily.len(), 1);
        assert_eq!(history.daily[0].date, "2026-10-19");
        assert!(history.models.is_empty() && history.efforts.is_empty());
    }

    #[test]
    fn short_ids_hide_most_of_the_account() {
        assert_eq!(
            short_id("e3c5dfb7-041e-4be0-8278-37296bca887c"),
            "e3c5…887c"
        );
        assert_eq!(short_id("abc"), "abc");
    }
}
