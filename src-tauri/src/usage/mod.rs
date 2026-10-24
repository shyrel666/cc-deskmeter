//! Offline token and cost accounting over Claude Code's local JSONL transcripts.
//! Reads the transcript folders found by `paths` (and `cleanupPeriodDays`
//! from the `settings.json` beside them), Claude Desktop's own
//! `plan-usage-history.json` (`quota`) and the local `pricing.json`; writes
//! only the aggregates-only history archive (`archive`) in the app's config
//! folder. Never touches the network.

mod aggregate;
mod archive;
mod parser;
mod paths;
mod pricing;
mod quota;
mod store;
mod types;

pub use archive::day_start;
pub(crate) use archive::write_atomic;
pub use paths::{Roots, display_dir};
pub use pricing::Pricing;
pub use quota::default_paths as quota_history_paths;
pub use store::{UsageStore, lock};
pub use types::{ArchiveSummary, ReadError, UsageData};
