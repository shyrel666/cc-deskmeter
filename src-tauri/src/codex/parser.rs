//! Parsing of Codex rollout lines into the records the ledger needs.
//!
//! Codex writes each record as `{"timestamp":…,"ordinal":…,"type":…,
//! "payload":{"type":…`, so its kind sits in the first bytes of even a
//! multi-megabyte line. Only the kinds below are parsed in full; tool output,
//! reasoning and messages (most of a rollout's bytes) are skipped unread.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use memchr::memmem;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::types::CodexTokens;

/// How far into a line its kinds and ordinal are looked for.
const HEAD_BYTES: usize = 512;
/// Characters of a prompt kept for a thread's title.
const PROMPT_CHARS: usize = 120;

/// The window and reset of one `rate_limits` entry (`primary`/`secondary`).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowReading {
    pub used_percent: f64,
    pub window_minutes: u64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// An account's official limits as one `token_count` reported them.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// `codex` for the plan's own limits; other buckets (`premium`) carry no windows.
    pub limit_id: Option<Arc<str>>,
    pub plan: Option<Arc<str>>,
    pub primary: Option<WindowReading>,
    pub secondary: Option<WindowReading>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    TurnContext {
        turn: Option<Arc<str>>,
        model: Option<Arc<str>>,
        effort: Option<Arc<str>>,
    },
    TurnStarted {
        turn: Option<Arc<str>>,
        context_window: Option<u64>,
    },
    /// `event_msg/token_count`: the thread's cumulative counter and the last
    /// response's usage (`info`), and the account's limits.
    TokenCount {
        total: Option<CodexTokens>,
        last: Option<CodexTokens>,
        context_window: Option<u64>,
        limits: Option<Arc<Limits>>,
    },
    /// `token_usage_record`: one response's usage, logged since Codex 0.153.
    Usage {
        response: Arc<str>,
        turn: Option<Arc<str>>,
        usage: CodexTokens,
    },
    /// The context was summarised; since Codex 0.153 with the summarising
    /// request's id, whose usage is a `token_usage_record` of its own.
    Compacted { response: Option<Arc<str>> },
    /// A subagent's own turn begins (`trigger_turn: true`).
    AgentTrigger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `ordinal` as Codex wrote it, else the line's index in the file.
    pub ordinal: u64,
    pub at: DateTime<Utc>,
    pub record: Record,
}

/// What the first `session_meta` of a rollout says about its thread.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeta {
    pub id: Arc<str>,
    /// When this rollout (page) was started.
    pub started: Option<DateTime<Utc>>,
    pub cwd: Option<String>,
    /// The thread this one was forked or spawned from; never itself.
    pub parent: Option<Arc<str>>,
    /// Spawned by an agent (`source.subagent`), a review agent included.
    pub subagent: bool,
    pub nickname: Option<String>,
    /// `creator_account_id`, logged since Codex 0.158.
    pub account: Option<Arc<str>>,
    /// Records before this ordinal replay the parent's history.
    pub history_start: Option<u64>,
    /// The rollout continues another page of the thread (`history_base`).
    pub continues: bool,
}

/// Everything read from one rollout so far. Plain rollouts are fed line by
/// line as they grow, so the state carries across reads.
#[derive(Debug, Default, Clone)]
pub struct FileLog {
    pub meta: Option<SessionMeta>,
    pub entries: Vec<Entry>,
    /// The file's first prompt, for threads `session_index.jsonl` does not name.
    pub first_prompt: Option<String>,
    /// Highest ordinal of any line, parsed or not.
    pub max_ordinal: Option<u64>,
    lines: u64,
    last_at: Option<DateTime<Utc>>,
}

impl FileLog {
    /// Changes whenever something the ledger reads does: lines of other
    /// kinds (most of a growing rollout) leave it alone.
    pub fn signature(&self) -> (usize, bool, bool, Option<u64>) {
        let start = self.meta.as_ref().and_then(|meta| meta.history_start);
        let started_own = start.filter(|start| self.max_ordinal.is_some_and(|max| max >= *start));
        (
            self.entries.len(),
            self.meta.is_some(),
            self.first_prompt.is_some(),
            started_own,
        )
    }

    /// Takes in one line (with or without its newline).
    pub fn ingest(&mut self, line: &[u8]) {
        let index = self.lines;
        self.lines += 1;
        let head = &line[..line.len().min(HEAD_BYTES)];
        let ordinal = head_ordinal(head).unwrap_or(index);
        self.max_ordinal = Some(self.max_ordinal.map_or(ordinal, |max| max.max(ordinal)));
        let mut kinds = kinds(head);
        let (top, sub, item) = (kinds.next(), kinds.next(), kinds.next());
        match (top, sub) {
            (Some(b"session_meta"), _) if self.meta.is_none() => self.meta = parse_meta(line),
            (Some(b"turn_context"), _) => self.push(ordinal, line, parse_context),
            (Some(b"token_usage_record"), _) => self.push(ordinal, line, parse_usage_record),
            (Some(b"compacted"), _) => self.push(ordinal, line, parse_compacted),
            (Some(b"inter_agent_communication_metadata" | b"inter_agent_communication"), _) => {
                self.push(ordinal, line, parse_trigger)
            }
            (Some(b"event_msg"), Some(b"token_count")) => self.push(ordinal, line, parse_count),
            (Some(b"event_msg"), Some(b"task_started" | b"turn_started")) => {
                self.push(ordinal, line, parse_started)
            }
            (Some(b"event_msg"), Some(b"item_completed"))
                if item == Some(b"UserMessage") && self.first_prompt.is_none() =>
            {
                self.first_prompt = parse_prompt(line);
            }
            _ => {}
        }
    }

    fn push(
        &mut self,
        ordinal: u64,
        line: &[u8],
        parse: impl FnOnce(&[u8]) -> Option<(Option<DateTime<Utc>>, Record)>,
    ) {
        let Some((at, record)) = parse(line) else {
            return;
        };
        // A record without a readable time takes its predecessor's.
        let Some(at) = at.or(self.last_at).or_else(|| self.meta.as_ref()?.started) else {
            return;
        };
        self.last_at = Some(at);
        self.entries.push(Entry {
            ordinal,
            at,
            record,
        });
    }
}

/// The values of the `"type":"…"` keys in `head`, in order.
fn kinds(head: &[u8]) -> impl Iterator<Item = &[u8]> {
    const KEY: &[u8] = br#""type":""#;
    memmem::find_iter(head, KEY).filter_map(move |at| {
        let value = &head[at + KEY.len()..];
        memchr::memchr(b'"', value).map(|end| &value[..end])
    })
}

fn head_ordinal(head: &[u8]) -> Option<u64> {
    const KEY: &[u8] = br#""ordinal":"#;
    let at = memmem::find(head, KEY)? + KEY.len();
    let digits = head[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    std::str::from_utf8(&head[at..at + digits])
        .ok()?
        .parse()
        .ok()
}

#[derive(Deserialize)]
struct Line<P> {
    timestamp: Option<Value>,
    payload: Option<P>,
}

fn parse_line<'a, P: Deserialize<'a>>(line: &'a [u8]) -> Option<(Option<DateTime<Utc>>, P)> {
    let line: Line<P> = serde_json::from_slice(line).ok()?;
    Some((line.timestamp.as_ref().and_then(time_of), line.payload?))
}

/// An RFC 3339 string, or Unix seconds or milliseconds.
fn time_of(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(text) = value.as_str() {
        return DateTime::parse_from_rfc3339(text.trim())
            .ok()
            .map(|time| time.with_timezone(&Utc));
    }
    let raw = value.as_i64()?;
    if raw > 10_000_000_000 {
        Utc.timestamp_millis_opt(raw).single()
    } else {
        Utc.timestamp_opt(raw, 0).single()
    }
}

fn shared(text: Option<String>) -> Option<Arc<str>> {
    text.map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .map(Arc::from)
}

/// A count that may be logged as an integer, a float or a numeric string.
fn count<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(value_u64))
}

fn value_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|n| *n >= 0.0)
                .map(|n| n.round() as u64)
        }),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

#[derive(Deserialize, Default)]
struct RawUsage {
    #[serde(default, deserialize_with = "count")]
    input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count")]
    cached_input_tokens: Option<u64>,
    /// In every count of the author's rollouts (Codex 0.142 on), always zero.
    #[serde(default, deserialize_with = "count")]
    cache_write_input_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count")]
    output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "count")]
    reasoning_output_tokens: Option<u64>,
}

impl RawUsage {
    /// Cached input is part of input and reasoning part of output; a log
    /// that says otherwise is clamped rather than trusted. Cache writes are
    /// kept as logged: whether input includes them is only assumed (see
    /// `pricing`).
    fn tokens(&self) -> CodexTokens {
        let input = self.input_tokens.unwrap_or(0);
        let output = self.output_tokens.unwrap_or(0);
        CodexTokens {
            input,
            cached: self.cached_input_tokens.unwrap_or(0).min(input),
            cache_write: self.cache_write_input_tokens.unwrap_or(0),
            output,
            reasoning: self.reasoning_output_tokens.unwrap_or(0).min(output),
        }
    }
}

#[derive(Deserialize)]
struct MetaPayload {
    id: Option<String>,
    timestamp: Option<Value>,
    cwd: Option<String>,
    forked_from_id: Option<String>,
    parent_thread_id: Option<String>,
    source: Option<Value>,
    agent_nickname: Option<String>,
    creator_account_id: Option<String>,
    #[serde(default, deserialize_with = "count")]
    subagent_history_start_ordinal: Option<u64>,
    history_base: Option<Value>,
}

fn parse_meta(line: &[u8]) -> Option<SessionMeta> {
    let (at, meta) = parse_line::<MetaPayload>(line)?;
    let id = shared(meta.id)?;
    let spawned_by = meta
        .source
        .as_ref()
        .and_then(|source| source.pointer("/subagent/thread_spawn/parent_thread_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let parent = shared(meta.forked_from_id)
        .or_else(|| shared(meta.parent_thread_id))
        .or_else(|| shared(spawned_by))
        .filter(|parent| *parent != id);
    Some(SessionMeta {
        started: meta.timestamp.as_ref().and_then(time_of).or(at),
        cwd: meta.cwd.filter(|cwd| !cwd.trim().is_empty()),
        parent,
        subagent: meta
            .source
            .as_ref()
            .is_some_and(|source| source.get("subagent").is_some()),
        nickname: meta.agent_nickname.filter(|name| !name.trim().is_empty()),
        account: shared(meta.creator_account_id),
        history_start: meta.subagent_history_start_ordinal,
        continues: meta.history_base.is_some(),
        id,
    })
}

#[derive(Deserialize)]
struct ContextPayload {
    turn_id: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    collaboration_mode: Option<Value>,
}

fn parse_context(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, context) = parse_line::<ContextPayload>(line)?;
    let setting = |key: &str| {
        context
            .collaboration_mode
            .as_ref()?
            .pointer(&format!("/settings/{key}"))?
            .as_str()
            .map(str::to_owned)
    };
    let record = Record::TurnContext {
        turn: shared(context.turn_id),
        model: shared(context.model).or_else(|| shared(setting("model"))),
        effort: shared(context.effort).or_else(|| shared(setting("reasoning_effort"))),
    };
    Some((at, record))
}

#[derive(Deserialize)]
struct StartedPayload {
    turn_id: Option<String>,
    #[serde(default, deserialize_with = "count")]
    model_context_window: Option<u64>,
}

fn parse_started(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, started) = parse_line::<StartedPayload>(line)?;
    let record = Record::TurnStarted {
        turn: shared(started.turn_id),
        context_window: started.model_context_window.filter(|window| *window > 0),
    };
    Some((at, record))
}

#[derive(Deserialize)]
struct CountPayload {
    info: Option<CountInfo>,
    rate_limits: Option<Value>,
}

#[derive(Deserialize)]
struct CountInfo {
    total_token_usage: Option<RawUsage>,
    last_token_usage: Option<RawUsage>,
    #[serde(default, deserialize_with = "count")]
    model_context_window: Option<u64>,
}

fn parse_count(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, count) = parse_line::<CountPayload>(line)?;
    let info = count.info;
    let limits = count
        .rate_limits
        .as_ref()
        .and_then(parse_limits)
        .map(Arc::new);
    let total = info
        .as_ref()
        .and_then(|info| info.total_token_usage.as_ref())
        .map(RawUsage::tokens);
    if total.is_none() && limits.is_none() {
        return None;
    }
    let record = Record::TokenCount {
        total,
        last: info
            .as_ref()
            .and_then(|info| info.last_token_usage.as_ref())
            .map(RawUsage::tokens),
        context_window: info
            .and_then(|info| info.model_context_window)
            .filter(|window| *window > 0),
        limits,
    };
    Some((at, record))
}

/// Lenient on purpose: an odd value here must not cost the line its usage.
fn parse_limits(value: &Value) -> Option<Limits> {
    let text = |key: &str| shared(value.get(key)?.as_str().map(str::to_owned));
    let window = |key: &str| -> Option<WindowReading> {
        let window = value.get(key)?;
        Some(WindowReading {
            used_percent: window.get("used_percent")?.as_f64()?,
            window_minutes: window.get("window_minutes").and_then(value_u64)?,
            resets_at: window.get("resets_at").and_then(time_of),
        })
    };
    Some(Limits {
        limit_id: text("limit_id"),
        plan: text("plan_type"),
        primary: window("primary"),
        secondary: window("secondary"),
    })
}

#[derive(Deserialize)]
struct UsageRecordPayload {
    response_id: Option<String>,
    turn_id: Option<String>,
    usage: Option<RawUsage>,
}

fn parse_usage_record(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, record) = parse_line::<UsageRecordPayload>(line)?;
    let record = Record::Usage {
        response: shared(record.response_id)?,
        turn: shared(record.turn_id),
        usage: record.usage?.tokens(),
    };
    Some((at, record))
}

#[derive(Deserialize)]
struct CompactedPayload {
    compaction_response_id: Option<String>,
}

fn parse_compacted(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, compacted) = parse_line::<CompactedPayload>(line)?;
    let record = Record::Compacted {
        response: shared(compacted.compaction_response_id),
    };
    Some((at, record))
}

#[derive(Deserialize)]
struct TriggerPayload {
    trigger_turn: Option<Value>,
}

fn parse_trigger(line: &[u8]) -> Option<(Option<DateTime<Utc>>, Record)> {
    let (at, trigger) = parse_line::<TriggerPayload>(line)?;
    (trigger.trigger_turn == Some(Value::Bool(true))).then_some((at, Record::AgentTrigger))
}

#[derive(Deserialize)]
struct ItemPayload {
    item: Option<PromptItem>,
}

#[derive(Deserialize)]
struct PromptItem {
    #[serde(default)]
    content: Vec<Value>,
}

fn parse_prompt(line: &[u8]) -> Option<String> {
    let (_, payload) = parse_line::<ItemPayload>(line)?;
    let text = payload
        .item?
        .content
        .iter()
        .filter_map(|part| part.get("text")?.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let first = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(first.chars().take(PROMPT_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(lines: &[&str]) -> FileLog {
        let mut log = FileLog::default();
        for line in lines {
            log.ingest(line.as_bytes());
        }
        log
    }

    const META: &str = r#"{"timestamp":"2026-10-12T14:13:54.318Z","ordinal":0,"type":"session_meta","payload":{"creator_account_id":"59d12126-8694-4542-868a-210300328189","session_id":"01a13073","id":"01a13076","forked_from_id":"01a13073","parent_thread_id":"01a13073","timestamp":"2026-10-12T14:13:54.214Z","cwd":"D:\\Pycharm Project\\claude-pulse","originator":"Codex Desktop","cli_version":"0.162.0-alpha.17.2","source":{"subagent":{"thread_spawn":{"parent_thread_id":"01a13073","depth":1,"agent_path":"/root/ui_audit","agent_nickname":"Mill","agent_role":null}}},"agent_nickname":"Mill","subagent_history_start_ordinal":12,"base_instructions":{"text":"You are Codex"}}}"#;

    #[test]
    fn reads_the_first_session_meta_only() {
        let parent = r#"{"timestamp":"2026-10-12T14:13:54.319Z","ordinal":1,"type":"session_meta","payload":{"id":"01a13073","timestamp":"2026-10-12T14:10:33.052Z","source":"vscode"}}"#;
        let log = log(&[META, parent]);
        let meta = log.meta.expect("meta");
        assert_eq!(&*meta.id, "01a13076");
        assert_eq!(meta.parent.as_deref(), Some("01a13073"));
        assert!(meta.subagent);
        assert_eq!(meta.nickname.as_deref(), Some("Mill"));
        assert_eq!(
            meta.account.as_deref(),
            Some("59d12126-8694-4542-868a-210300328189")
        );
        assert_eq!(meta.history_start, Some(12));
        assert_eq!(
            meta.started.unwrap().to_rfc3339(),
            "2026-10-12T14:13:54.214+00:00"
        );
        assert_eq!(log.max_ordinal, Some(1));
    }

    #[test]
    fn a_thread_that_names_itself_as_parent_has_none() {
        let meta = r#"{"timestamp":"2026-06-26T11:48:50.158Z","type":"session_meta","payload":{"id":"019f03c2","parent_thread_id":"019f03c2","timestamp":"2026-06-26T11:48:50.084Z","source":{"subagent":{"other":"guardian"}}}}"#;
        let meta = log(&[meta]).meta.unwrap();
        assert_eq!(meta.parent, None);
        assert!(meta.subagent);
    }

    #[test]
    fn parses_usage_counts_and_limits() {
        let count = r#"{"timestamp":"2026-10-19T07:49:30.917Z","ordinal":22,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":37975,"cached_input_tokens":29696,"cache_write_input_tokens":0,"output_tokens":780,"reasoning_output_tokens":201,"total_tokens":38755},"last_token_usage":{"input_tokens":20373,"cached_input_tokens":17408,"output_tokens":601,"reasoning_output_tokens":201,"total_tokens":20974},"model_context_window":258400},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":1.0,"window_minutes":300,"resets_at":1792414073},"secondary":{"used_percent":25.0,"window_minutes":10080,"resets_at":1792892364},"credits":{"has_credits":false},"plan_type":"plus","rate_limit_reached_type":null}}}"#;
        let record = r#"{"timestamp":"2026-10-19T07:49:48.937Z","ordinal":26,"type":"token_usage_record","payload":{"thread_id":"t","turn_id":"turn-1","response_id":"resp_1","usage":{"input_tokens":20373,"cached_input_tokens":17408,"cache_write_input_tokens":1024,"output_tokens":601,"reasoning_output_tokens":201,"total_tokens":20974}}}"#;
        let log = log(&[count, record]);
        assert_eq!(log.entries.len(), 2);
        let Record::TokenCount {
            total,
            last,
            context_window,
            limits,
        } = &log.entries[0].record
        else {
            panic!("{:?}", log.entries[0]);
        };
        assert_eq!(log.entries[0].ordinal, 22);
        assert_eq!(
            *total,
            Some(CodexTokens {
                input: 37975,
                cached: 29696,
                cache_write: 0,
                output: 780,
                reasoning: 201
            })
        );
        assert_eq!(last.unwrap().total(), 20974);
        assert_eq!(last.unwrap().cache_write, 0, "not logged");
        assert_eq!(*context_window, Some(258_400));
        let limits = limits.as_ref().unwrap();
        assert_eq!(limits.limit_id.as_deref(), Some("codex"));
        assert_eq!(limits.plan.as_deref(), Some("plus"));
        let primary = limits.primary.as_ref().unwrap();
        assert_eq!(primary.window_minutes, 300);
        assert_eq!(primary.used_percent, 1.0);
        assert_eq!(primary.resets_at.unwrap().timestamp(), 1_792_414_073);
        assert_eq!(limits.secondary.as_ref().unwrap().window_minutes, 10080);
        assert_eq!(
            log.entries[1].record,
            Record::Usage {
                response: "resp_1".into(),
                turn: Some("turn-1".into()),
                usage: CodexTokens {
                    cache_write: 1024,
                    ..last.unwrap()
                },
            }
        );
    }

    #[test]
    fn a_premium_bucket_and_null_info_still_parse() {
        let premium = r#"{"timestamp":"2026-10-17T02:35:52.595Z","ordinal":883,"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"premium","limit_name":null,"primary":null,"secondary":null,"plan_type":"plus"}}}"#;
        let log = log(&[premium]);
        let Record::TokenCount { total, limits, .. } = &log.entries[0].record else {
            panic!();
        };
        assert_eq!(*total, None);
        let limits = limits.as_ref().unwrap();
        assert_eq!(limits.limit_id.as_deref(), Some("premium"));
        assert!(limits.primary.is_none() && limits.secondary.is_none());
    }

    #[test]
    fn reads_turns_compactions_triggers_and_prompts() {
        let lines = [
            r#"{"timestamp":"2026-10-19T07:49:21.133Z","ordinal":1,"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1","started_at":1792396161,"model_context_window":258400}}"#,
            r#"{"timestamp":"2026-10-19T07:49:22.975Z","ordinal":9,"type":"turn_context","payload":{"turn_id":"turn-1","cwd":"D:\\x","model":"gpt-6-astra","collaboration_mode":{"mode":"default","settings":{"model":"gpt-6-astra","reasoning_effort":"max"}},"effort":"max"}}"#,
            r#"{"timestamp":"2026-10-19T07:49:22.990Z","ordinal":10,"type":"event_msg","payload":{"type":"item_completed","thread_id":"t","turn_id":"turn-1","item":{"type":"UserMessage","id":"u","content":[{"type":"text","text":"\n  审查我当前未提交的代码变更\nsecond line","text_elements":[]}]}}}"#,
            r#"{"timestamp":"2026-10-19T08:10:22.460Z","ordinal":266,"type":"compacted","payload":{"message":"","replacement_history":[{"type":"message"}],"compaction_response_id":"resp_c"}}"#,
            r#"{"timestamp":"2026-10-19T08:10:23.000Z","ordinal":267,"type":"inter_agent_communication_metadata","payload":{"trigger_turn":false}}"#,
            r#"{"timestamp":"2026-10-19T08:10:24.000Z","ordinal":268,"type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}"#,
            r#"{"timestamp":"2026-10-19T08:10:25.000Z","ordinal":269,"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"\"type\":\"token_count\""}]}}"#,
        ];
        let log = log(&lines);
        let records: Vec<_> = log.entries.iter().map(|entry| &entry.record).collect();
        assert_eq!(
            records,
            [
                &Record::TurnStarted {
                    turn: Some("turn-1".into()),
                    context_window: Some(258_400)
                },
                &Record::TurnContext {
                    turn: Some("turn-1".into()),
                    model: Some("gpt-6-astra".into()),
                    effort: Some("max".into())
                },
                &Record::Compacted {
                    response: Some("resp_c".into())
                },
                &Record::AgentTrigger,
            ]
        );
        assert_eq!(
            log.first_prompt.as_deref(),
            Some("审查我当前未提交的代码变更")
        );
        assert_eq!(log.max_ordinal, Some(269));
    }

    #[test]
    fn old_turn_contexts_fall_back_to_collaboration_settings() {
        let line = r#"{"timestamp":"2026-06-26T11:48:50Z","type":"turn_context","payload":{"turn_id":"t","collaboration_mode":{"settings":{"model":"gpt-5.5","reasoning_effort":"xhigh"}}}}"#;
        assert_eq!(
            log(&[line]).entries[0].record,
            Record::TurnContext {
                turn: Some("t".into()),
                model: Some("gpt-5.5".into()),
                effort: Some("xhigh".into())
            }
        );
    }

    #[test]
    fn lines_without_ordinals_count_by_position_and_bad_lines_are_skipped() {
        let lines = [
            "not json at all",
            r#"{"timestamp":"2026-06-26T11:48:53.346Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":"8000","output_tokens":630.0},"last_token_usage":{"input_tokens":8000,"cached_input_tokens":9000,"output_tokens":630,"reasoning_output_tokens":700}}}}"#,
            r#"{"timestamp":"bad","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9000,"output_tokens":700}}}}"#,
        ];
        let log = log(&lines);
        assert_eq!(log.max_ordinal, Some(2));
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].ordinal, 1);
        let Record::TokenCount { total, last, .. } = &log.entries[0].record else {
            panic!();
        };
        assert_eq!(
            total.unwrap().total(),
            8630,
            "numbers written as strings or floats"
        );
        let last = last.unwrap();
        assert_eq!(
            (last.cached, last.reasoning),
            (8000, 630),
            "clamped to their totals"
        );
        assert_eq!(
            log.entries[1].at, log.entries[0].at,
            "an unreadable time takes the previous record's"
        );
    }
}
