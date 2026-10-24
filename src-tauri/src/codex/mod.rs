//! Offline token, cost and rate-limit accounting over Codex's local rollouts
//! (`$CODEX_HOME/sessions` and `archived_sessions`, plain or `.zst`), with
//! thread names from `session_index.jsonl` and costs at OpenAI's API prices
//! (`pricing`, plus the local `pricing.json` the Claude side reads too).
//! Reads nothing else under the Codex home but the account id in
//! `auth.json` (`signin`), writes nothing there and never touches the
//! network.

mod accounts;
mod ledger;
mod parser;
mod paths;
mod pricing;
mod reader;
mod report;
mod signin;
mod store;
mod types;

pub use paths::CodexHome;
pub use store::{CodexStore, lock};
pub use types::{CodexData, CodexHistory};
