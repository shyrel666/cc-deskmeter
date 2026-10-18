//! Offline token and cost accounting over Claude Code's local JSONL transcripts.
//! Only reads files under `~/.claude/projects`; never touches the network.

mod aggregate;
mod parser;
mod pricing;
mod store;
mod types;

pub use store::{lock, UsageStore};
pub use types::UsageData;
