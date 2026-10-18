//! Parsing of single Claude Code JSONL transcript lines.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::types::TokenCounts;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLine {
    uuid: Option<String>,
    parent_uuid: Option<String>,
    timestamp: Option<String>,
    request_id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation: Option<RawCacheCreation>,
    speed: Option<String>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
}

/// The fields of a line that carries `message.usage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUsage {
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub message_id: Option<String>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub tokens: TokenCounts,
    /// Part of `tokens.cache_creation` written with the 1-hour TTL.
    pub cache_write_1h: u64,
    pub fast: bool,
    pub complete: bool,
}

/// Returns `None` for lines without usage, without a valid timestamp, with
/// all-zero usage (e.g. synthetic error messages), or that are not valid JSON.
pub fn parse_line(line: &str) -> Option<ParsedUsage> {
    // Most lines are user/tool-result records; skip them before the JSON parse.
    if !line.contains("\"usage\"") {
        return None;
    }
    let raw: RawLine = serde_json::from_str(line).ok()?;
    let message = raw.message?;
    let usage = message.usage?;
    let timestamp = DateTime::parse_from_rfc3339(raw.timestamp.as_deref()?)
        .ok()?
        .with_timezone(&Utc);

    let tokens = TokenCounts {
        input: usage.input_tokens.unwrap_or(0),
        output: usage.output_tokens.unwrap_or(0),
        cache_creation: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read: usage.cache_read_input_tokens.unwrap_or(0),
    };
    if tokens.total() == 0 {
        return None;
    }
    let cache_write_1h = usage
        .cache_creation
        .and_then(|c| c.ephemeral_1h_input_tokens)
        .unwrap_or(0)
        .min(tokens.cache_creation);

    Some(ParsedUsage {
        uuid: raw.uuid,
        parent_uuid: raw.parent_uuid,
        message_id: message.id,
        request_id: raw.request_id,
        session_id: raw.session_id,
        cwd: raw.cwd,
        timestamp,
        model: message.model.unwrap_or_else(|| "unknown".to_owned()),
        tokens,
        cache_write_1h,
        fast: usage.speed.as_deref() == Some("fast"),
        complete: message.stop_reason.is_some(),
    })
}

/// Only top-level fields participate in request association. serde skips the
/// contents of messages/tool results, regardless of field order or whitespace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContext {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
}

impl RecordContext {
    pub fn input_time(&self) -> Option<DateTime<Utc>> {
        matches!(self.kind.as_deref(), Some("user" | "attachment"))
            .then_some(self.timestamp)
            .flatten()
    }
}

pub fn parse_context(line: &str) -> Option<RecordContext> {
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT: &str = r#"{"parentUuid":"c32a","isSidechain":false,"message":{"model":"claude-opus-5-5","id":"msg_011","type":"message","role":"assistant","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":22224,"cache_read_input_tokens":44148,"output_tokens":447,"cache_creation":{"ephemeral_1h_input_tokens":22224,"ephemeral_5m_input_tokens":0},"speed":"standard"}},"requestId":"req_011","type":"assistant","timestamp":"2026-10-17T14:06:22.358Z","cwd":"D:\\Pycharm Project\\claude-pulse","sessionId":"f056a8b2"}"#;

    #[test]
    fn parses_assistant_usage_line() {
        let parsed = parse_line(ASSISTANT).expect("usage line");
        assert_eq!(parsed.message_id.as_deref(), Some("msg_011"));
        assert_eq!(parsed.request_id.as_deref(), Some("req_011"));
        assert_eq!(parsed.session_id.as_deref(), Some("f056a8b2"));
        assert_eq!(
            parsed.cwd.as_deref(),
            Some(r"D:\Pycharm Project\claude-pulse")
        );
        assert_eq!(parsed.model, "claude-opus-5-5");
        assert_eq!(parsed.tokens.total(), 2 + 447 + 22224 + 44148);
        assert_eq!(parsed.cache_write_1h, 22224);
        assert!(!parsed.fast);
        assert_eq!(
            parsed.timestamp.to_rfc3339(),
            "2026-10-17T14:06:22.358+00:00"
        );
    }

    #[test]
    fn skips_lines_without_usable_usage() {
        assert!(
            parse_line(r#"{"type":"user","message":{"role":"user","content":"usage?"}}"#).is_none()
        );
        assert!(parse_line(r#"{"type":"summary","summary":"x"}"#).is_none());
        assert!(parse_line("not json \"usage\"").is_none());
        let zero = r#"{"timestamp":"2026-10-17T14:06:22Z","message":{"id":"m","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        assert!(parse_line(zero).is_none());
        let no_ts = r#"{"message":{"id":"m","usage":{"input_tokens":5}}}"#;
        assert!(parse_line(no_ts).is_none());
    }

    #[test]
    fn request_context_uses_only_top_level_fields() {
        let user = r#"{"type":"user","toolUseResult":{"timestamp":"2020-01-01T00:00:00Z"},"uuid":"u","timestamp": "2026-10-18T01:51:59.025Z"}"#;
        assert_eq!(
            parse_context(user)
                .unwrap()
                .input_time()
                .map(|t| t.to_rfc3339())
                .as_deref(),
            Some("2026-10-18T01:51:59.025+00:00")
        );
        for kind in ["progress", "system", "summary", "assistant"] {
            let line = format!(r#"{{"type":"{kind}","timestamp":"2026-10-18T01:51:59Z"}}"#);
            assert!(parse_context(&line).unwrap().input_time().is_none());
        }
        assert!(parse_context(r#"{"timestamp":"not a time"}"#).is_none());
    }
}
