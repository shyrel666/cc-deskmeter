//! Plan quota (5-hour and 7-day usage %) from Claude Desktop's own local log.
//!
//! Claude Desktop records the official plan usage about every 15 minutes while
//! it runs, in `<config dir>/Claude/plan-usage-history.json` (see `default_paths`):
//! `{"version":2,"samples":[{"t":<ms>,"org":"…","u":{"fh":23,"sd":41}}]}`
//! (`fh` = 5-hour %, `sd` = 7-day %). This is a local file, read like the
//! transcripts: no network, no credentials. The organization id only keeps
//! accounts apart and never leaves this module; the archive gets a hash of it.
//!
//! Between two samples the quota is estimated from local spend:
//! `estimate = last official % + k × quota-weighted spend since that sample`.
//! A backtest against real samples found that the quota follows API-equivalent
//! cost far better than raw tokens (5h MAE 0.4 pp vs 3.7 pp), with cache
//! traffic weighted below its list price (see `WEIGHTS`). `k` (% per dollar)
//! depends on the plan and is learned from consecutive samples, weighting
//! recent pairs more.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use super::pricing::{self, ModelRates};
use super::types::TokenCounts;

pub const HISTORY_FILE: &str = "plan-usage-history.json";
const SUPPORTED_VERSION: u32 = 2;

/// The shares of their list price at which cache reads and writes count
/// toward the quota. Archived spend and calibrations keep the weights they
/// were computed with, so changing these recomputes or retires them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Weights {
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Cache reads: fitted on one day of samples, confirmed on the next. Cache
/// writes: at full price, a window that opened by rewriting a long session's
/// expired cache came out about 2 pp high; at 0.75 the walk-forward 5-hour
/// error fell from 0.51 to 0.39 pp on the later half of four days of samples
/// (the weight picked on the earlier half).
pub const WEIGHTS: Weights = Weights {
    cache_read: 0.625,
    cache_write: 0.75,
};
/// Consecutive samples further apart than this mean Desktop was closed or
/// asleep; such a pair may hide a reset and is not used for calibration.
const MAX_SAMPLE_GAP: TimeDelta = TimeDelta::minutes(32);
/// Recent sample pairs dominate `k`, so it follows a plan change within hours.
const FIVE_HOUR_HALF_LIFE_HOURS: f64 = 6.0;
/// The 7-day value rises about a point per 15 minutes of use, too coarse for
/// that: after an idle night the one pair since then set `k` (20% high). With
/// a day, several share it, and a plan change still shows within a day. In a
/// walk-forward backtest the error 4 to 48 hours past the latest sample
/// (Desktop closed) fell from 0.9 to 0.6–0.7 pp; between samples it held.
const SEVEN_DAY_HALF_LIFE_HOURS: f64 = 24.0;
/// Below this much (unweighted) spend across usable pairs, `k` is too noisy.
const MIN_CALIBRATION_SPEND: f64 = 1.0;
const MIN_CALIBRATION_PAIRS: usize = 2;
/// The 5-hour limit is projected at the pace of this span: short enough that
/// a burst shows quickly and an idle stretch stops the countdown.
const LIMIT_PACE_SPAN: TimeDelta = TimeDelta::minutes(30);
/// Desktop samples every 15 minutes; older means it stopped sampling.
const STALE_AFTER: TimeDelta = TimeDelta::minutes(20);
/// Error band: ±(BASE + SHARE × estimated increase) percentage points.
const BAND_BASE: f64 = 1.0;
const BAND_SHARE: f64 = 0.1;
const FIVE_HOURS: TimeDelta = TimeDelta::hours(5);
const SEVEN_DAYS: TimeDelta = TimeDelta::days(7);
/// A drop in the 7-day value of at least this many points opens a new weekly
/// period: Claude's own reset, or one made by hand.
const WEEK_RESET_DROP: f64 = 2.0;
/// The weekly pace looks back this far, so a whole day's rhythm is in it.
const PACE_WINDOW: TimeDelta = TimeDelta::hours(24);
/// Within the first hour of a period a pace would be noise.
const MIN_PACE_WINDOW: TimeDelta = TimeDelta::hours(1);
/// Weekly trend readings are thinned to at most one per this interval.
const WEEK_POINT_SPACING: TimeDelta = TimeDelta::hours(1);
/// A predicted limit further away than this is left out.
const LIMIT_HORIZON: TimeDelta = TimeDelta::days(14);
/// Attribution rows below this many percentage points are left out.
const MIN_SHARE: f64 = 0.05;
/// The weekly share of a full 5-hour window needs at least a full window's
/// worth of 5-hour points; below that the integer readings are too coarse.
const MIN_WINDOW_POINTS: f64 = 100.0;

/// Spend that counts toward the plan quota, in API-equivalent dollars, of
/// responses in one price tier (see `pricing::tier_cost_usd`).
pub fn quota_weight(
    rates: &ModelRates,
    tokens: &TokenCounts,
    cache_write_1h: u64,
    fast: bool,
    long_context: bool,
) -> f64 {
    let rates = ModelRates {
        cache_read: rates.cache_read * WEIGHTS.cache_read,
        cache_write_5m: rates.cache_write_5m * WEIGHTS.cache_write,
        cache_write_1h: rates.cache_write_1h * WEIGHTS.cache_write,
        ..*rates
    };
    pricing::tier_cost_usd(&rates, tokens, cache_write_1h, fast, long_context)
}

/// Where Claude Desktop may keep the file: `<config dir>/Claude` (`%APPDATA%`
/// on Windows, `~/Library/Application Support` on macOS, `~/.config` on
/// Linux). The Microsoft Store (MSIX) build on Windows writes `%APPDATA%`
/// into its package instead, which only Desktop and its child processes see
/// as `%APPDATA%`: `%LOCALAPPDATA%/Packages/Claude_<id>/LocalCache/Roaming/Claude`.
/// The most recently written candidate wins.
pub fn default_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = dirs::config_dir()
        .map(|dir| dir.join("Claude").join(HISTORY_FILE))
        .into_iter()
        .collect();
    #[cfg(windows)]
    if let Some(packages) = dirs::data_local_dir().map(|dir| dir.join("Packages"))
        && let Ok(entries) = std::fs::read_dir(packages)
    {
        let mut packaged: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("Claude_"))
            .map(|entry| {
                entry
                    .path()
                    .join("LocalCache")
                    .join("Roaming")
                    .join("Claude")
                    .join(HISTORY_FILE)
            })
            .collect();
        packaged.sort();
        paths.extend(packaged);
    }
    paths
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub at: DateTime<Utc>,
    pub five_hour: f64,
    pub seven_day: f64,
}

#[derive(Deserialize)]
struct RawHistory {
    version: u32,
    samples: Vec<RawSample>,
}

#[derive(Deserialize)]
struct RawSample {
    t: i64,
    org: Option<String>,
    u: RawUsage,
}

#[derive(Deserialize)]
struct RawUsage {
    fh: f64,
    sd: f64,
}

#[cfg(test)]
pub fn parse(json: &str) -> Result<Vec<Sample>, String> {
    parse_history(json).map(|(_, samples)| samples)
}

/// A stable tag for an organization id that does not reveal it, so the
/// archive can keep accounts apart (`""` for samples without one).
fn account_tag(org: Option<&str>) -> String {
    org.map_or_else(String::new, |org| {
        format!("{:08x}", crc32fast::hash(org.as_bytes()))
    })
}

/// The samples of the account that wrote the latest one, oldest first, with
/// that account's tag. Errors on an unknown format, so a changed file never
/// yields wrong numbers.
fn parse_history(json: &str) -> Result<(String, Vec<Sample>), String> {
    let raw: RawHistory = serde_json::from_str(json).map_err(|err| err.to_string())?;
    if raw.version != SUPPORTED_VERSION {
        return Err(format!("unsupported version {}", raw.version));
    }
    let mut samples: Vec<(Option<String>, Sample)> = raw
        .samples
        .into_iter()
        .filter(|s| [s.u.fh, s.u.sd].iter().all(|v| v.is_finite() && *v >= 0.0))
        .filter_map(|s| {
            let at = DateTime::from_timestamp_millis(s.t)?;
            Some((
                s.org,
                Sample {
                    at,
                    five_hour: s.u.fh,
                    seven_day: s.u.sd,
                },
            ))
        })
        .collect();
    samples.sort_by_key(|(_, s)| s.at);
    let Some((latest_org, _)) = samples.last().cloned() else {
        return Ok((String::new(), Vec::new()));
    };
    let account = account_tag(latest_org.as_deref());
    let samples = samples
        .into_iter()
        .filter(|(org, _)| *org == latest_org)
        .map(|(_, s)| s)
        .collect();
    Ok((account, samples))
}

type Stamp = (PathBuf, u64, Option<SystemTime>);

/// Reads the newest of the candidate history files, re-parsing only when the
/// chosen file, its size or its mtime changed, or the previous read failed.
#[derive(Debug, Default)]
pub struct QuotaSource {
    candidates: Vec<PathBuf>,
    stamp: Option<Stamp>,
    account: String,
    samples: Vec<Sample>,
    last_error: Option<String>,
}

impl QuotaSource {
    pub fn new(candidates: Vec<PathBuf>) -> Self {
        Self {
            candidates,
            ..Self::default()
        }
    }

    /// The most recently modified candidate that exists. Re-checked on every
    /// reload, so a file that appears later (or a reinstall) is picked up.
    fn current(&self) -> Option<Stamp> {
        self.candidates
            .iter()
            .filter_map(|path| {
                let meta = std::fs::metadata(path).ok()?;
                meta.is_file()
                    .then(|| (path.clone(), meta.len(), meta.modified().ok()))
            })
            .max_by_key(|(_, _, modified)| *modified)
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The tag of the account the samples belong to, while there are any.
    pub fn account(&self) -> Option<&str> {
        (!self.samples.is_empty()).then_some(self.account.as_str())
    }

    /// Returns whether the samples changed. A missing or unreadable file, or
    /// one in an unknown format, clears them (the quota is then hidden).
    /// Failed reads are retried even when the file's metadata is unchanged.
    pub fn reload(&mut self) -> bool {
        let stamp = self.current();
        if stamp == self.stamp {
            return false;
        }
        let loaded = stamp
            .as_ref()
            .map_or_else(|| Ok(Default::default()), |(path, _, _)| load(path));
        let (account, samples) = match loaded {
            Ok(parsed) => {
                self.stamp = stamp;
                self.last_error = None;
                parsed
            }
            Err(err) => {
                // A sharing violation can go away without changing size or mtime.
                self.stamp = None;
                if self.last_error.as_ref() != Some(&err) {
                    eprintln!("cc-deskmeter: ignoring {err}");
                }
                self.last_error = Some(err);
                Default::default()
            }
        };
        let changed = samples != self.samples || account != self.account;
        self.account = account;
        self.samples = samples;
        changed
    }
}

fn load(path: &Path) -> Result<(String, Vec<Sample>), String> {
    std::fs::read_to_string(path)
        .map_err(|err| err.to_string())
        .and_then(|text| parse_history(&text))
        .map_err(|err| format!("{}: {err}", path.display()))
}

// ---------------------------------------------------------------------------
// Estimation
// ---------------------------------------------------------------------------

/// Quota-weighted spend per response, ordered by response time.
#[derive(Debug, Default)]
pub struct Spend {
    times: Vec<DateTime<Utc>>,
    /// `cumulative[i]` = spend of the first `i` responses.
    cumulative: Vec<f64>,
}

impl Spend {
    pub fn new(mut points: Vec<(DateTime<Utc>, f64)>) -> Self {
        points.sort_by_key(|(at, _)| *at);
        let mut cumulative = Vec::with_capacity(points.len() + 1);
        cumulative.push(0.0);
        for (_, usd) in &points {
            cumulative.push(cumulative.last().copied().unwrap_or(0.0) + usd);
        }
        Self {
            times: points.into_iter().map(|(at, _)| at).collect(),
            cumulative,
        }
    }

    /// Spend of responses in `(from, to]`.
    pub fn between(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
        if to <= from {
            return 0.0;
        }
        let index = |t: DateTime<Utc>| self.times.partition_point(|at| *at <= t);
        self.cumulative[index(to)] - self.cumulative[index(from)]
    }
}

/// The local 5-hour block the estimate can lean on for resets.
#[derive(Debug, Clone, Copy)]
pub struct LocalBlock {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaStatus {
    /// When Claude Desktop recorded the latest official value.
    pub sampled_at: DateTime<Utc>,
    /// Desktop has stopped sampling (likely closed): the estimate rests on
    /// this machine's logs only and misses usage elsewhere.
    pub stale: bool,
    /// `None` while the window that opened after the latest sample cannot be
    /// estimated yet (too few samples to convert its spend): unknown, not 0.
    pub five_hour: Option<QuotaWindow>,
    /// `None` once the latest sample is older than the 7-day window itself.
    pub seven_day: Option<QuotaWindow>,
    /// The current weekly period; present exactly when `seven_day` is.
    pub week: Option<WeekStatus>,
    /// `k` fitted on these samples now, not taken from the archive.
    #[serde(skip)]
    pub fitted: PercentPerUsd,
}

/// `k` per window, in % per quota-weighted dollar.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PercentPerUsd {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

/// One official 7-day reading.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekPoint {
    pub at: DateTime<Utc>,
    pub percent: f64,
}

/// The part of the weekly limit one project or model used on this machine.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekShare {
    /// Project folder or model id.
    pub id: String,
    /// Project name, or the model id.
    pub name: String,
    /// Percentage points of the weekly limit.
    pub percent: f64,
}

/// The weekly period the latest sample belongs to: its readings, the recent
/// pace and what used it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekStatus {
    /// Just before the latest drop in the 7-day value (Claude's reset, or one
    /// made by hand), else the first reading of the last 7 days.
    pub period_start: DateTime<Utc>,
    /// `period_start` is an observed drop, so the period opened near 0 %.
    pub reset_observed: bool,
    /// Official readings since the drop, at most one per hour, oldest first.
    pub points: Vec<WeekPoint>,
    /// Where the pace is measured from: a day back, or the period start.
    pub pace_since: Option<DateTime<Utc>>,
    /// Percentage points per hour; `None` within the period's first hour.
    pub pace_per_hour: Option<f64>,
    /// When the estimate reaches 100 % at that pace, if within two weeks.
    pub limit_at: Option<DateTime<Utc>>,
    /// This machine's use since `period_start`, largest first. Empty until
    /// spend can be converted into % (see `QuotaWindow::calibrated`).
    pub projects: Vec<WeekShare>,
    pub models: Vec<WeekShare>,
    /// Weekly percentage points that a full 5-hour window uses up, from the
    /// readings of the last 7 days. `None` until they cover a full window.
    pub window_share: Option<f64>,
    /// `k` of the 7-day window over the whole period (unweighted, so the
    /// shares add up to the period's readings), in % per quota-weighted
    /// dollar; the recent `k` while the period has too few pairs.
    #[serde(skip)]
    pub percent_per_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// Latest official value, as recorded.
    pub official_percent: f64,
    /// Official value plus estimated use since, clamped to 0..=100.
    pub estimate_percent: f64,
    pub low_percent: f64,
    pub high_percent: f64,
    /// False until enough sample pairs exist to convert spend into %;
    /// the estimate then equals the official value, or 0 once the window ended.
    pub calibrated: bool,
    /// The window reset since the sample, per the local 5-hour block.
    pub reset_since_sample: bool,
    /// End of the local 5-hour block (5-hour window only).
    pub resets_at: Option<DateTime<Utc>>,
    /// At the current pace, minutes until 100%; `None` when idle, uncalibrated
    /// or not before the window resets (5-hour window only).
    pub minutes_to_limit: Option<i64>,
}

#[derive(Clone, Copy)]
enum Window {
    FiveHour,
    SevenDay,
}

impl Window {
    fn value(self, sample: &Sample) -> f64 {
        match self {
            Self::FiveHour => sample.five_hour,
            Self::SevenDay => sample.seven_day,
        }
    }
}

/// `k` in % per dollar: ratio over consecutive sample pairs that are close
/// together and show no reset (the value did not drop), weighted by recency
/// when `half_life` (hours) is given.
///
/// Ages count back from the latest sample, not from now: the time since then
/// would scale every weight alike and cancel out of the ratio, so leaving it
/// out makes the fit the same whenever it is made (see `fit`).
fn calibrate(
    samples: &[Sample],
    spend: &Spend,
    window: Window,
    half_life: Option<f64>,
) -> Option<f64> {
    let latest = samples.last()?.at;
    let (mut gain, mut cost, mut raw_cost, mut pairs) = (0.0, 0.0, 0.0, 0);
    for pair in samples.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let delta = window.value(b) - window.value(a);
        if b.at - a.at > MAX_SAMPLE_GAP || delta < 0.0 {
            continue;
        }
        let x = spend.between(a.at, b.at);
        if x <= 0.0 {
            continue;
        }
        let age_hours = (latest - b.at).as_seconds_f64() / 3600.0;
        let weight = half_life.map_or(1.0, |half_life| 0.5_f64.powf(age_hours / half_life));
        gain += weight * delta;
        cost += weight * x;
        raw_cost += x;
        pairs += 1;
    }
    (pairs >= MIN_CALIBRATION_PAIRS && raw_cost >= MIN_CALIBRATION_SPEND && cost > 0.0)
        .then(|| gain / cost)
}

/// `k` of both windows, fitted on `samples` with recent pairs weighted more.
/// It depends on the samples and the spend between them only, so it holds
/// until either changes, however much later it is used.
pub fn fit(samples: &[Sample], spend: &Spend) -> PercentPerUsd {
    PercentPerUsd {
        five_hour: calibrate(
            samples,
            spend,
            Window::FiveHour,
            Some(FIVE_HOUR_HALF_LIFE_HOURS),
        ),
        seven_day: calibrate(
            samples,
            spend,
            Window::SevenDay,
            Some(SEVEN_DAY_HALF_LIFE_HOURS),
        ),
    }
}

fn window_estimate(base: f64, official: f64, increase: f64, calibrated: bool) -> QuotaWindow {
    let estimate = (base + increase).clamp(0.0, 100.0);
    let band = if increase > 0.0 {
        BAND_BASE + BAND_SHARE * increase
    } else {
        0.0
    };
    QuotaWindow {
        official_percent: official,
        estimate_percent: estimate,
        low_percent: (estimate - band).max(base.min(100.0)),
        high_percent: (estimate + band).min(100.0),
        calibrated,
        reset_since_sample: false,
        resets_at: None,
        minutes_to_limit: None,
    }
}

/// Official quota with an estimate of the use since the latest sample.
/// `fitted` is `fit(samples, spend)`, which callers may keep between calls.
/// `block` is the open local 5-hour block, `last_block_end` the end of the
/// latest local block (open or not). `archived` stands in for a window's `k`
/// while the samples are too few to fit it (the transcripts of their spend
/// may be gone).
pub fn estimate(
    samples: &[Sample],
    spend: &Spend,
    fitted: PercentPerUsd,
    block: Option<LocalBlock>,
    last_block_end: Option<DateTime<Utc>>,
    archived: PercentPerUsd,
    now: DateTime<Utc>,
) -> Option<QuotaStatus> {
    let anchor = *samples.last()?;
    let stale = now - anchor.at > STALE_AFTER;

    // 5-hour window. Claude starts it with the first request after the last
    // one ended, which the local block mirrors; a block that opened after the
    // sample, or ended since it, means the sampled window is over.
    let k5 = fitted.five_hour.or(archived.five_hour);
    let new_block = block.filter(|b| b.start > anchor.at);
    let ended = block.is_none() && last_block_end.is_some_and(|end| end > anchor.at && end <= now);
    let expired = now - anchor.at > FIVE_HOURS && block.is_none_or(|b| b.start > anchor.at);
    let reset = new_block.is_some() || ended || expired;
    let (base, since) = match new_block {
        Some(b) => (0.0, b.start),
        None if reset => (0.0, now),
        None => (anchor.five_hour, anchor.at),
    };
    let increase = k5.map_or(0.0, |k| k * spend.between(since, now));
    let mut five_hour = window_estimate(base, anchor.five_hour, increase, k5.is_some());
    five_hour.reset_since_sample = reset;
    five_hour.resets_at = block.map(|b| b.end);
    if let (Some(k), Some(b)) = (k5, block) {
        // Always over the whole span, even minutes into a block: over those
        // minutes alone, one burst (a long session rebuilding its cache) set
        // the limit hours too soon. Spend before the block is the same pace.
        let per_minute =
            k * spend.between(now - LIMIT_PACE_SPAN, now) / LIMIT_PACE_SPAN.num_minutes() as f64;
        if per_minute > 0.0 {
            let left = (100.0 - five_hour.estimate_percent) / per_minute;
            // Compared in minutes: at a crawl, `left` lies millions of years
            // ahead (or is infinite), beyond any time that could be built.
            let until_reset = (b.end - now).as_seconds_f64() / 60.0;
            if left < until_reset {
                five_hour.minutes_to_limit = Some(left.ceil() as i64);
            }
        }
    }
    // Use in a window that opened after the sample is known only through
    // local spend, which takes `k`.
    let five_hour = (k5.is_some() || new_block.is_none()).then_some(five_hour);

    // 7-day window: its reset time is unknown locally, so the estimate simply
    // carries on until the next sample shows the drop.
    let k7 = fitted.seven_day.or(archived.seven_day);
    let seven_day = (now - anchor.at <= SEVEN_DAYS).then(|| {
        let increase = k7.map_or(0.0, |k| k * spend.between(anchor.at, now));
        window_estimate(anchor.seven_day, anchor.seven_day, increase, k7.is_some())
    });
    let week = seven_day
        .as_ref()
        .map(|window| week_status(samples, spend, window.estimate_percent, k7, now));

    Some(QuotaStatus {
        sampled_at: anchor.at,
        stale,
        five_hour,
        seven_day,
        week,
        fitted,
    })
}

/// The weekly period of the latest sample, paced from the official readings
/// to `estimate` (the current 7-day estimate). Needs a sample within 7 days.
fn week_status(
    samples: &[Sample],
    spend: &Spend,
    estimate: f64,
    k7: Option<f64>,
    now: DateTime<Utc>,
) -> WeekStatus {
    // The latest drop opens the period. Across a long gap (Desktop closed)
    // the reset cannot be placed, so the period starts at the next reading.
    let drop = samples
        .windows(2)
        .rposition(|pair| weekly_reset(&pair[0], &pair[1]).is_some());
    let (mut start, mut first, mut observed) = match drop {
        Some(i) if samples[i + 1].at - samples[i].at <= MAX_SAMPLE_GAP => {
            (samples[i].at, i + 1, true)
        }
        Some(i) => (samples[i + 1].at, i + 1, true),
        None => (samples[0].at, 0, false),
    };
    // No period outlasts a week: an older start means a reset went unseen.
    let week_ago = now - SEVEN_DAYS;
    if start < week_ago {
        first = samples
            .partition_point(|s| s.at < week_ago)
            .min(samples.len() - 1);
        start = samples[first].at;
        observed = false;
    }
    let period = &samples[first..];
    // An observed reset placed before the first reading opened at ~0 %.
    let opening = if observed && start < period[0].at {
        0.0
    } else {
        period[0].seven_day
    };

    let mut points: Vec<WeekPoint> = Vec::new();
    for (index, sample) in period.iter().enumerate() {
        if index + 1 == period.len()
            || points
                .last()
                .is_none_or(|p| sample.at - p.at >= WEEK_POINT_SPACING)
        {
            points.push(WeekPoint {
                at: sample.at,
                percent: sample.seven_day,
            });
        }
    }

    // From the latest reading a day old, or the period start, to now.
    let (since, base) = period
        .iter()
        .rev()
        .find(|s| now - s.at >= PACE_WINDOW)
        .map_or((start, opening), |s| (s.at, s.seven_day));
    let elapsed_hours = (now - since).num_seconds() as f64 / 3600.0;
    let pace =
        (now - since >= MIN_PACE_WINDOW).then(|| ((estimate - base) / elapsed_hours).max(0.0));
    let limit_at = if estimate >= 100.0 {
        Some(now)
    } else {
        pace.filter(|pace| *pace > 0.0)
            .map(|pace| (100.0 - estimate) / pace)
            .filter(|hours| *hours <= LIMIT_HORIZON.num_hours() as f64)
            .and_then(|hours| TimeDelta::try_seconds((hours * 3600.0) as i64))
            .and_then(|left| now.checked_add_signed(left))
    };

    WeekStatus {
        period_start: start,
        reset_observed: observed,
        points,
        pace_since: pace.is_some().then_some(since),
        pace_per_hour: pace,
        limit_at,
        projects: Vec::new(),
        models: Vec::new(),
        window_share: window_share(samples, now),
        // A recent `k` applied to the whole week's spend leaves its drift as
        // a remainder that looks like use elsewhere.
        percent_per_usd: calibrate(period, spend, Window::SevenDay, None).or(k7),
    }
}

/// Whether the 7-day value dropped from `a` to the next reading `b` far
/// enough to open a new weekly period, and if so whether the two are close
/// enough together to place the reset.
pub fn weekly_reset(a: &Sample, b: &Sample) -> Option<bool> {
    (b.seven_day <= a.seven_day - WEEK_RESET_DROP).then(|| b.at - a.at <= MAX_SAMPLE_GAP)
}

/// How far the 7-day value rises per 5-hour point, times 100: summed over
/// close sample pairs where neither value dropped (a reset) nor sits at the
/// limit (where it stops rising). Both values come from Claude's readings, so
/// models and caching weigh into them alike and the ratio stays steady.
fn window_share(samples: &[Sample], now: DateTime<Utc>) -> Option<f64> {
    let (mut five_hour, mut seven_day) = (0.0, 0.0);
    for pair in samples.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let (rise_5h, rise_7d) = (b.five_hour - a.five_hour, b.seven_day - a.seven_day);
        if now - a.at > SEVEN_DAYS
            || b.at - a.at > MAX_SAMPLE_GAP
            || rise_5h < 0.0
            || rise_7d < 0.0
            || b.five_hour >= 100.0
            || b.seven_day >= 100.0
        {
            continue;
        }
        five_hour += rise_5h;
        seven_day += rise_7d;
    }
    (five_hour >= MIN_WINDOW_POINTS).then(|| 100.0 * seven_day / five_hour)
}

/// Attribution rows from `(id, name, quota-weighted dollars)`, converted
/// with `k` (% per dollar), largest first.
pub fn week_shares(
    spend: impl IntoIterator<Item = (String, String, f64)>,
    k: f64,
) -> Vec<WeekShare> {
    let mut shares: Vec<WeekShare> = spend
        .into_iter()
        .map(|(id, name, usd)| WeekShare {
            id,
            name,
            percent: k * usd,
        })
        .filter(|share| share.percent >= MIN_SHARE)
        .collect();
    shares.sort_by(|a, b| {
        b.percent
            .total_cmp(&a.percent)
            .then_with(|| a.name.cmp(&b.name))
    });
    shares
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + TimeDelta::minutes(minutes)
    }

    fn sample(minutes: i64, fh: f64, sd: f64) -> Sample {
        Sample {
            at: at(minutes),
            five_hour: fh,
            seven_day: sd,
        }
    }

    fn no_spend() -> Spend {
        Spend::new(Vec::new())
    }

    /// Fitted on the spot, without an archived calibration.
    fn estimate(
        samples: &[Sample],
        spend: &Spend,
        block: Option<LocalBlock>,
        last_block_end: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<QuotaStatus> {
        super::estimate(
            samples,
            spend,
            fit(samples, spend),
            block,
            last_block_end,
            PercentPerUsd::default(),
            now,
        )
    }

    /// $1 of spend in the middle of every 15-minute step from 0 to `until`.
    fn steady_spend(until: i64) -> Vec<(DateTime<Utc>, f64)> {
        (0..until / 15).map(|i| (at(i * 15 + 7), 1.0)).collect()
    }

    #[test]
    fn parses_the_latest_account_and_rejects_unknown_formats() {
        let json = r#"{"version":2,"samples":[
            {"t":1792219169847,"org":"a","u":{"fh":0,"sd":0}},
            {"t":1792219963054,"org":"b","u":{"fh":7,"sd":3}},
            {"t":1792220862958,"org":"a","u":{"fh":11,"sd":2}}]}"#;
        let samples = parse(json).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].five_hour, 11.0);
        assert!(samples[0].at < samples[1].at);

        assert!(parse(r#"{"version":3,"samples":[]}"#).is_err());
        assert!(parse(r#"{"version":2,"samples":[{"t":1,"u":{"fh":"x","sd":1}}]}"#).is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"version":2,"samples":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn reads_the_newest_candidate_and_follows_changes() {
        let dir = std::env::temp_dir().join(format!("cc-deskmeter-quota-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let roaming = dir.join("roaming.json");
        let packaged = dir.join("packaged.json");
        let missing = dir.join("missing.json");
        let history = |fh: u32| {
            format!(
                r#"{{"version":2,"samples":[{{"t":1792219169847,"org":"a","u":{{"fh":{fh},"sd":1}}}}]}}"#
            )
        };
        let set_age = |path: &Path, minutes_ago: u64| {
            let time = SystemTime::now() - std::time::Duration::from_secs(minutes_ago * 60);
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        std::fs::write(&roaming, history(10)).unwrap();
        std::fs::write(&packaged, history(20)).unwrap();
        set_age(&roaming, 60);
        set_age(&packaged, 5);

        let mut source = QuotaSource::new(vec![missing, roaming.clone(), packaged]);
        assert!(source.reload());
        assert_eq!(source.samples()[0].five_hour, 20.0);
        assert!(!source.reload(), "unchanged files are not re-read");

        // The other copy becomes the newest one.
        std::fs::write(&roaming, history(30)).unwrap();
        set_age(&roaming, 1);
        assert!(source.reload());
        assert_eq!(source.samples()[0].five_hour, 30.0);

        // A file in an unknown format hides the quota instead of keeping stale numbers.
        std::fs::write(&roaming, r#"{"version":9,"samples":[]}"#).unwrap();
        set_age(&roaming, 0);
        assert!(source.reload());
        assert!(source.samples().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_read_is_retried_even_with_the_same_stamp() {
        let dir =
            std::env::temp_dir().join(format!("cc-deskmeter-quota-retry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(HISTORY_FILE);
        let valid =
            r#"{"version":2,"samples":[{"t":1792219169847,"org":"a","u":{"fh":23,"sd":41}}]}"#;
        std::fs::write(&path, valid.replace("\"version\":2", "\"version\":9")).unwrap();
        let modified = path.metadata().unwrap().modified().unwrap();
        let mut source = QuotaSource::new(vec![path.clone()]);
        let stamp = source.current();
        assert!(!source.reload());
        assert!(source.samples().is_empty());
        std::fs::write(&path, valid).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(source.current(), stamp);
        assert!(source.reload());
        assert_eq!(source.samples()[0].five_hour, 23.0);
        assert!(!source.reload(), "successful reads are still cached");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sharing_violation_does_not_cache_a_failed_read() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir =
            std::env::temp_dir().join(format!("cc-deskmeter-quota-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(HISTORY_FILE);
        std::fs::write(
            &path,
            r#"{"version":2,"samples":[{"t":1792219169847,"org":"a","u":{"fh":23,"sd":41}}]}"#,
        )
        .unwrap();
        let mut source = QuotaSource::new(vec![path.clone()]);
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let stamp = source.current();
        assert!(
            stamp.is_some(),
            "metadata remains readable while content is locked"
        );
        assert!(!source.reload());
        assert!(source.samples().is_empty());
        drop(locked);
        assert_eq!(source.current(), stamp);
        assert!(source.reload());
        assert_eq!(source.samples().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quota_weight_discounts_cache_traffic_only() {
        let rates = pricing::rates_for("claude-opus-5-5").unwrap();
        let read = TokenCounts {
            cache_read: 1_000_000,
            ..Default::default()
        };
        assert!((quota_weight(&rates, &read, 0, false, false) - 0.125).abs() < 1e-12);
        // 1M writes, half with the 1-hour TTL: $2.50 + $4.00 at list price.
        let write = TokenCounts {
            cache_creation: 1_000_000,
            ..Default::default()
        };
        assert!((quota_weight(&rates, &write, 500_000, false, false) - 4.875).abs() < 1e-12);
        let output = TokenCounts {
            output: 1_000_000,
            ..Default::default()
        };
        assert!(
            (quota_weight(&rates, &output, 0, false, false)
                - pricing::cost_usd(&rates, &output, 0, false))
            .abs()
                < 1e-12
        );
    }

    #[test]
    fn spend_sums_half_open_intervals() {
        let spend = Spend::new(vec![(at(10), 1.0), (at(5), 2.0), (at(20), 4.0)]);
        assert_eq!(spend.between(at(5), at(20)), 5.0);
        assert_eq!(spend.between(at(0), at(5)), 2.0);
        assert_eq!(spend.between(at(20), at(5)), 0.0);
    }

    #[test]
    fn calibration_skips_resets_and_gaps_and_needs_enough_pairs() {
        // 2.5 % per dollar, a reset (drop) and a long gap in between.
        let samples = [
            sample(0, 0.0, 0.0),
            sample(15, 2.5, 1.0),
            sample(30, 5.0, 1.0),
            sample(45, 1.0, 1.0),  // drop: ignored
            sample(120, 9.0, 2.0), // 75-minute gap: ignored
            sample(135, 11.5, 2.0),
        ];
        let spend = Spend::new(steady_spend(135));
        let k = calibrate(
            &samples,
            &spend,
            Window::FiveHour,
            Some(FIVE_HOUR_HALF_LIFE_HOURS),
        )
        .unwrap();
        assert!((k - 2.5).abs() < 1e-9, "{k}");
        assert!(
            calibrate(
                &samples[..2],
                &spend,
                Window::FiveHour,
                Some(FIVE_HOUR_HALF_LIFE_HOURS)
            )
            .is_none()
        );
    }

    #[test]
    fn calibration_follows_recent_pairs() {
        // 1 %/$ for a day, then 4 %/$ (e.g. a plan change) for six hours.
        let mut samples = Vec::new();
        let mut value = 0.0;
        for i in 0..=120 {
            samples.push(sample(i * 15, value, 0.0));
            value += if i < 96 { 1.0 } else { 4.0 };
        }
        let spend = Spend::new(steady_spend(121 * 15));
        let k = calibrate(
            &samples,
            &spend,
            Window::FiveHour,
            Some(FIVE_HOUR_HALF_LIFE_HOURS),
        )
        .unwrap();
        // Unweighted, the ratio would be 1.5.
        assert!(k > 2.4 && k < 4.0, "{k}");
    }

    #[test]
    fn the_weekly_fit_outlasts_an_idle_night() {
        // 0.4 %/$ for eight hours, read as whole points; nine idle hours; then
        // one pair that reads 0.6 %/$ (3 points for $5).
        let mut samples: Vec<Sample> = (0..=32)
            .map(|i| sample(i * 15, 0.0, (0.4 * i as f64).floor()))
            .collect();
        samples.push(sample(1020, 0.0, 12.0));
        samples.push(sample(1035, 0.0, 15.0));
        let mut points: Vec<_> = (1..=32).map(|i| (at(i * 15 - 8), 1.0)).collect();
        points.push((at(1027), 5.0));
        let k = fit(&samples, &Spend::new(points)).seven_day.unwrap();
        // Unweighted, 0.405; a 6-hour half-life gave 0.466.
        assert!(k > 0.40 && k < 0.42, "{k}");
    }

    #[test]
    fn the_fit_is_the_same_whenever_it_is_made() {
        // Desktop stamps samples to the millisecond. The ratio changes halfway
        // (2 %/$, then 3 %/$), so the recency weights shape the fit.
        let samples: Vec<Sample> = (0..=16)
            .map(|i| {
                let value = if i <= 8 {
                    2.0 * i as f64
                } else {
                    16.0 + 3.0 * (i - 8) as f64
                };
                Sample {
                    at: at(i * 15) + TimeDelta::milliseconds(i * 137 % 1000),
                    five_hour: value,
                    seven_day: value / 8.0,
                }
            })
            .collect();
        let spend = Spend::new(steady_spend(16 * 15));
        let fitted = fit(&samples, &spend);
        let k = fitted.five_hour.unwrap();
        assert!(k > 2.0 && k < 3.0, "{k}");
        assert!(fitted.seven_day.is_some());
        let start = at(16 * 15 + 1);
        for later in [0, 1, 250, 999, 1_500, 17_321, 600_000] {
            let now = start + TimeDelta::milliseconds(later);
            let q = estimate(&samples, &spend, None, None, now).unwrap();
            assert_eq!(q.fitted, fitted, "{later} ms later");
        }
    }

    #[test]
    fn a_crawling_pace_has_no_time_to_the_limit() {
        // A tiny `k` and a hundredth of a cent in the last half hour put 100 %
        // millions of years ahead, past any representable time; a vanishing
        // spend puts it infinitely far.
        let samples = [sample(0, 40.0, 10.0)];
        let block = LocalBlock {
            start: at(-60),
            end: at(240),
        };
        let archived = PercentPerUsd {
            five_hour: Some(2.4e-6),
            seven_day: Some(2.4e-6),
        };
        for spent in [1e-4, 1e-300] {
            let spend = Spend::new(vec![(at(5), spent)]);
            let q = super::estimate(
                &samples,
                &spend,
                fit(&samples, &spend),
                Some(block),
                Some(block.end),
                archived,
                at(10),
            )
            .unwrap();
            let fh = five_hour(&q);
            assert!(fh.calibrated);
            assert_eq!(fh.minutes_to_limit, None, "${spent}");
        }
    }

    fn calibrated_samples() -> Vec<Sample> {
        // 2 % and 0.25 % per dollar.
        (0..=8)
            .map(|i| sample(i * 15, 2.0 * i as f64, 0.25 * i as f64))
            .collect()
    }

    fn five_hour(q: &QuotaStatus) -> &QuotaWindow {
        q.five_hour.as_ref().expect("a 5-hour estimate")
    }

    #[test]
    fn estimates_use_since_the_sample_with_a_band() {
        let samples = calibrated_samples();
        let mut points = steady_spend(120);
        points.push((at(125), 5.0));
        let spend = Spend::new(points);
        let block = LocalBlock {
            start: at(100),
            end: at(400),
        };
        let q = estimate(&samples, &spend, Some(block), Some(block.end), at(130)).unwrap();
        assert!(!q.stale);
        let fh = five_hour(&q);
        assert!(fh.calibrated && !fh.reset_since_sample);
        assert_eq!(fh.official_percent, 16.0);
        assert!((fh.estimate_percent - 26.0).abs() < 1e-9);
        assert!((fh.high_percent - 28.0).abs() < 1e-9);
        assert!((fh.low_percent - 24.0).abs() < 1e-9);
        assert_eq!(fh.resets_at, Some(at(400)));
        // $6 in the last 30 minutes => 0.4 % per minute => 74 % left in 185 min,
        // which is before the block ends.
        assert_eq!(fh.minutes_to_limit, Some(185));
        let early = LocalBlock {
            start: at(0),
            end: at(300),
        };
        let q = estimate(&samples, &spend, Some(early), Some(early.end), at(130)).unwrap();
        assert_eq!(
            five_hour(&q).minutes_to_limit,
            None,
            "the window resets first"
        );
        let sd = q.seven_day.unwrap();
        assert!((sd.estimate_percent - 3.25).abs() < 1e-9);
    }

    #[test]
    fn a_burst_early_in_a_block_is_paced_over_the_whole_span() {
        // 2 % per dollar; the block opens at 130 with a $15 burst.
        let samples = calibrated_samples();
        let mut points = steady_spend(120);
        points.push((at(133), 15.0));
        let spend = Spend::new(points);
        let block = LocalBlock {
            start: at(130),
            end: at(430),
        };
        let q = estimate(&samples, &spend, Some(block), Some(block.end), at(150)).unwrap();
        let fh = five_hour(&q);
        assert!((fh.estimate_percent - 30.0).abs() < 1e-9);
        // 30 % over 30 minutes, not over the block's 20: 70 % left in 70 min.
        assert_eq!(fh.minutes_to_limit, Some(70));
    }

    #[test]
    fn a_new_local_block_restarts_the_five_hour_estimate() {
        let samples = calibrated_samples();
        let mut points = steady_spend(120);
        points.push((at(400), 3.0));
        let spend = Spend::new(points);
        let block = LocalBlock {
            start: at(390),
            end: at(690),
        };
        let q = estimate(&samples, &spend, Some(block), Some(block.end), at(410)).unwrap();
        assert!(q.stale);
        let fh = five_hour(&q);
        assert!(fh.reset_since_sample);
        assert!((fh.estimate_percent - 6.0).abs() < 1e-9);
        assert_eq!(fh.official_percent, 16.0);
    }

    #[test]
    fn an_ended_block_or_an_old_sample_means_the_window_is_over() {
        let samples = calibrated_samples();
        let spend = Spend::new(steady_spend(120));
        let q = estimate(&samples, &spend, None, Some(at(200)), at(210)).unwrap();
        assert!(five_hour(&q).reset_since_sample);
        assert_eq!(five_hour(&q).estimate_percent, 0.0);
        // No local block info at all: only the 5-hour limit itself tells.
        let q = estimate(&samples, &spend, None, None, at(120 + 301)).unwrap();
        assert!(five_hour(&q).reset_since_sample && five_hour(&q).estimate_percent == 0.0);
        let q = estimate(&samples, &spend, None, None, at(150)).unwrap();
        assert!(!five_hour(&q).reset_since_sample && five_hour(&q).estimate_percent == 16.0);
    }

    #[test]
    fn uncalibrated_estimates_stay_at_the_official_value() {
        let samples = [sample(0, 40.0, 10.0)];
        let spend = Spend::new(vec![(at(5), 3.0), (at(70), 2.0)]);
        let block = LocalBlock {
            start: at(-60),
            end: at(240),
        };
        let q = estimate(&samples, &spend, Some(block), Some(block.end), at(10)).unwrap();
        let fh = five_hour(&q);
        assert!(!fh.calibrated);
        assert_eq!(fh.estimate_percent, 40.0);
        assert_eq!(fh.low_percent, 40.0);
        assert_eq!(fh.high_percent, 40.0);
        assert!(fh.minutes_to_limit.is_none());
        // A window that opened after the sample has no official value to stay
        // at, and its spend cannot be converted yet: unknown, not 0 %.
        let new = LocalBlock {
            start: at(60),
            end: at(360),
        };
        let q = estimate(&samples, &spend, Some(new), Some(new.end), at(75)).unwrap();
        assert!(q.five_hour.is_none());
        assert!(q.seven_day.is_some());
        // An ended window holds no use, calibrated or not.
        let q = estimate(&samples, &spend, None, Some(at(30)), at(75)).unwrap();
        assert_eq!(five_hour(&q).estimate_percent, 0.0);
        assert!(estimate(&[], &spend, None, None, at(10)).is_none());
    }

    #[test]
    fn an_archived_calibration_converts_spend_until_the_samples_can() {
        let samples = [sample(0, 40.0, 10.0)];
        let spend = Spend::new(vec![(at(5), 3.0)]);
        let block = LocalBlock {
            start: at(-60),
            end: at(240),
        };
        let archived = PercentPerUsd {
            five_hour: Some(2.0),
            seven_day: Some(0.5),
        };
        let q = super::estimate(
            &samples,
            &spend,
            fit(&samples, &spend),
            Some(block),
            Some(block.end),
            archived,
            at(10),
        )
        .unwrap();
        let fh = five_hour(&q);
        assert!(fh.calibrated);
        assert!((fh.estimate_percent - 46.0).abs() < 1e-9);
        assert!((q.seven_day.unwrap().estimate_percent - 11.5).abs() < 1e-9);
        assert_eq!(q.fitted, PercentPerUsd::default(), "nothing was fitted");
        // Once the samples allow a fit, it wins.
        let samples = calibrated_samples();
        let spend = Spend::new(steady_spend(120));
        let fitted = fit(&samples, &spend);
        let q = super::estimate(&samples, &spend, fitted, None, None, archived, at(120)).unwrap();
        assert!((q.fitted.five_hour.unwrap() - 2.0).abs() < 1e-9);
        assert!((q.fitted.seven_day.unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn estimates_clamp_at_the_limit_and_drop_a_week_old_seven_day_value() {
        let samples = calibrated_samples();
        let mut points = steady_spend(120);
        points.push((at(125), 100.0));
        let spend = Spend::new(points);
        let block = LocalBlock {
            start: at(0),
            end: at(300),
        };
        let q = estimate(&samples, &spend, Some(block), Some(block.end), at(126)).unwrap();
        assert_eq!(five_hour(&q).estimate_percent, 100.0);
        assert_eq!(five_hour(&q).high_percent, 100.0);
        let q = estimate(
            &samples,
            &spend,
            None,
            None,
            at(120) + SEVEN_DAYS + TimeDelta::minutes(1),
        )
        .unwrap();
        assert!(q.seven_day.is_none() && q.week.is_none());
    }

    #[test]
    fn the_week_opens_at_the_latest_drop_and_is_paced_to_the_estimate() {
        // 30 %, a reset to 1 % at minute 60, then a point every hour.
        let mut samples = vec![
            sample(0, 0.0, 28.0),
            sample(15, 0.0, 29.0),
            sample(30, 0.0, 30.0),
            sample(45, 0.0, 30.0),
        ];
        samples.extend((0..=48).map(|i| sample(60 + i * 15, 0.0, 1.0 + i as f64 * 0.25)));
        let now = at(790);
        let week = week_status(&samples, &no_spend(), 13.5, Some(0.5), now);
        assert!(week.reset_observed);
        assert_eq!(week.period_start, at(45), "just before the drop");
        // Hourly readings from the drop on, the latest always included.
        let points: Vec<_> = week.points.iter().map(|p| (p.at, p.percent)).collect();
        assert_eq!(points.len(), 13);
        assert_eq!((points[0], points[12]), ((at(60), 1.0), (at(780), 13.0)));
        // Younger than a day: paced from ~0 % at the period start.
        assert_eq!(week.pace_since, Some(at(45)));
        let pace = week.pace_per_hour.unwrap();
        assert!((pace - 13.5 / (745.0 / 60.0)).abs() < 1e-9);
        let hours_left = (week.limit_at.unwrap() - now).num_seconds() as f64 / 3600.0;
        assert!((hours_left - 86.5 / pace).abs() < 1e-3);
        assert_eq!(week.percent_per_usd, Some(0.5));
    }

    #[test]
    fn the_week_handles_day_old_readings_long_gaps_and_unseen_resets() {
        // A day-old reading sets the pace: hourly readings, 1 point an hour.
        let samples: Vec<_> = (0..=30).map(|h| sample(h * 60, 0.0, h as f64)).collect();
        let week = week_status(&samples, &no_spend(), 30.5, None, at(30 * 60));
        assert!(!week.reset_observed);
        assert_eq!(week.period_start, at(0));
        assert_eq!(week.pace_since, Some(at(6 * 60)));
        assert!((week.pace_per_hour.unwrap() - 24.5 / 24.0).abs() < 1e-9);

        // Desktop closed across the reset: the period starts at the next reading.
        let samples = [sample(0, 0.0, 40.0), sample(600, 0.0, 5.0)];
        let week = week_status(&samples, &no_spend(), 6.0, None, at(720));
        assert!(week.reset_observed);
        assert_eq!(week.period_start, at(600));
        assert!((week.pace_per_hour.unwrap() - 1.0 / 2.0).abs() < 1e-9);

        // A drop more than a week old: some later reset went unseen.
        let day = 24 * 60;
        let samples = [
            sample(0, 0.0, 50.0),
            sample(15, 0.0, 1.0),
            sample(2 * day, 0.0, 10.0),
            sample(8 * day - 6, 0.0, 40.0),
        ];
        let week = week_status(&samples, &no_spend(), 40.0, None, at(8 * day));
        assert!(!week.reset_observed);
        assert_eq!(week.period_start, at(2 * day));
        assert_eq!(week.points.len(), 2);

        // The first hour of a period has no pace yet.
        let week = week_status(
            &[sample(0, 0.0, 30.0), sample(15, 0.0, 1.0)],
            &no_spend(),
            2.0,
            None,
            at(40),
        );
        assert_eq!(
            (week.pace_per_hour, week.pace_since, week.limit_at),
            (None, None, None)
        );
    }

    #[test]
    fn the_limit_is_left_out_when_idle_or_far_off_and_is_now_once_reached() {
        let samples: Vec<_> = (0..=30).map(|h| sample(h * 60, 0.0, 6.0)).collect();
        let idle = week_status(&samples, &no_spend(), 6.0, None, at(30 * 60));
        assert_eq!((idle.pace_per_hour, idle.limit_at), (Some(0.0), None));
        // 0.1 point a day: years to go.
        let slow = week_status(&samples, &no_spend(), 6.1, None, at(30 * 60));
        assert!(slow.pace_per_hour.unwrap() > 0.0 && slow.limit_at.is_none());
        let spent = week_status(&samples, &no_spend(), 100.0, None, at(30 * 60));
        assert_eq!(spent.limit_at, Some(at(30 * 60)));
    }

    #[test]
    fn week_attribution_uses_the_whole_period_not_the_recent_k() {
        // Half a point per dollar early on, a quarter lately: the period's own
        // ratio (6 points over $16) keeps the shares adding up to the readings.
        let samples: Vec<_> = (0..=16)
            .map(|i| {
                sample(
                    i * 15,
                    0.0,
                    if i <= 8 {
                        i as f64 * 0.5
                    } else {
                        4.0 + (i - 8) as f64 * 0.25
                    },
                )
            })
            .collect();
        let spend = Spend::new(steady_spend(240));
        let week = week_status(&samples, &spend, 6.0, Some(0.25), at(240));
        assert!((week.percent_per_usd.unwrap() - 0.375).abs() < 1e-9);
    }

    #[test]
    fn a_full_window_is_measured_in_weekly_points_once_readings_cover_one() {
        // 10 five-hour points per weekly point would make a full window 10 %.
        let rising = |n: i64| -> Vec<Sample> {
            (0..=n)
                .map(|i| sample(i * 15, (i * 10 % 100) as f64, i as f64))
                .collect()
        };
        // 9 steps of 10 points each: not yet a full window.
        assert_eq!(window_share(&rising(9), at(9 * 15)), None);
        // Past 10 steps the 5-hour value wraps (a reset), and that pair is
        // skipped; so are pairs across a long gap or a weekly drop.
        let mut samples = rising(20);
        samples.push(sample(20 * 15 + 60, 50.0, 30.0));
        samples.push(sample(20 * 15 + 75, 60.0, 2.0));
        let share = window_share(&samples, at(20 * 15 + 75)).unwrap();
        assert!((share - 10.0).abs() < 1e-9);
        // Readings older than a week are left out.
        assert_eq!(window_share(&samples, at(8 * 24 * 60)), None);
    }

    #[test]
    fn week_shares_convert_spend_and_drop_crumbs() {
        let shares = week_shares(
            [
                ("a".into(), "Alpha".into(), 2.0),
                ("b".into(), "Beta".into(), 10.0),
                ("c".into(), "Gamma".into(), 0.01),
            ],
            0.5,
        );
        let rows: Vec<_> = shares
            .iter()
            .map(|s| (s.name.as_str(), s.percent))
            .collect();
        assert_eq!(rows, [("Beta", 5.0), ("Alpha", 1.0)]);
    }

    /// Replays a sanitized export (`responses.csv` + `samples.json`, as made for
    /// the backtest) and prints the walk-forward error of the 5-hour estimate:
    /// `CC_DESKMETER_BACKTEST=<dir> cargo test -- --ignored --nocapture replay_backtest`
    #[test]
    #[ignore = "needs a local backtest export"]
    fn replay_backtest() {
        let dir =
            PathBuf::from(std::env::var("CC_DESKMETER_BACKTEST").expect("CC_DESKMETER_BACKTEST"));
        let samples_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("samples.json")).unwrap())
                .unwrap();
        let samples: Vec<Sample> = samples_json["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Sample {
                at: DateTime::from_timestamp_millis(s["t"].as_i64().unwrap()).unwrap(),
                five_hour: s["fh"].as_f64().unwrap(),
                seven_day: s["sd"].as_f64().unwrap(),
            })
            .collect();
        let csv = std::fs::read_to_string(dir.join("responses.csv")).unwrap();
        let points: Vec<(DateTime<Utc>, f64)> = csv
            .lines()
            .skip(1)
            .map(|line| {
                let c: Vec<&str> = line.split(',').collect();
                let num = |i: usize| c[i].parse::<u64>().unwrap();
                let tokens = TokenCounts {
                    input: num(5),
                    output: num(6),
                    cache_creation: num(7),
                    cache_read: num(9),
                };
                let rates = pricing::rates_for(c[3]).unwrap();
                let at = DateTime::parse_from_rfc3339(c[1])
                    .unwrap()
                    .with_timezone(&Utc);
                let long = rates
                    .long_context
                    .is_some_and(|tier| tokens.prompt() > tier.threshold);
                (
                    at,
                    quota_weight(&rates, &tokens, num(8), c[4] == "fast", long),
                )
            })
            .collect();
        let spend = Spend::new(points);
        let (mut errors, mut covered) = (Vec::new(), 0);
        for i in 1..samples.len() {
            let (prev, next) = (samples[i - 1], samples[i]);
            if next.at - prev.at > MAX_SAMPLE_GAP || next.five_hour < prev.five_hour {
                continue;
            }
            let Some(k) = calibrate(
                &samples[..i],
                &spend,
                Window::FiveHour,
                Some(FIVE_HOUR_HALF_LIFE_HOURS),
            ) else {
                continue;
            };
            let increase = k * spend.between(prev.at, next.at);
            let error = prev.five_hour + increase - next.five_hour;
            covered += usize::from(error.abs() <= BAND_BASE + BAND_SHARE * increase);
            errors.push(error);
        }
        let mae = errors.iter().map(|e| e.abs()).sum::<f64>() / errors.len() as f64;
        println!(
            "5h: n={} MAE={mae:.3} band coverage={}/{}",
            errors.len(),
            covered,
            errors.len()
        );
        assert!(mae < 1.0);
    }
}
