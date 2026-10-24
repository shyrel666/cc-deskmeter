//! Where Codex keeps its rollouts: `$CODEX_HOME` (default `~/.codex`), with
//! live threads under `sessions/YYYY/MM/DD/` and archived ones under
//! `archived_sessions/`. Codex compresses older rollouts in place
//! (`rollout-….jsonl` becomes `rollout-….jsonl.zst`), so both are read.
//! Of the files beside them only `auth.json` is opened, for its account id
//! (`signin`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::usage::{ReadError, display_dir};

pub const HOME_ENV: &str = "CODEX_HOME";
const SESSIONS: &str = "sessions";
const ARCHIVED: &str = "archived_sessions";
const SESSION_INDEX: &str = "session_index.jsonl";
const PLAIN: &str = ".jsonl";
const COMPRESSED: &str = ".jsonl.zst";

#[derive(Debug, Clone)]
pub struct CodexHome {
    pub dir: PathBuf,
    /// The raw `CODEX_HOME` value when it chose `dir`.
    from_env: Option<String>,
    home: Option<PathBuf>,
}

impl From<PathBuf> for CodexHome {
    fn from(dir: PathBuf) -> Self {
        Self {
            dir,
            from_env: None,
            home: None,
        }
    }
}

impl CodexHome {
    pub fn discover() -> Self {
        Self::resolve(std::env::var_os(HOME_ENV), dirs::home_dir())
    }

    fn resolve(env: Option<OsString>, home: Option<PathBuf>) -> Self {
        // A blank value is treated as unset, as Codex does.
        let from_env = env
            .map(|value| value.to_string_lossy().trim().to_owned())
            .filter(|value| !value.is_empty());
        let dir = match (&from_env, home.as_deref()) {
            (Some(value), home) => expand_home(value, home),
            (None, Some(home)) => home.join(".codex"),
            (None, None) => PathBuf::from(".codex"),
        };
        Self {
            dir,
            from_env,
            home,
        }
    }

    /// Folders holding rollouts, live ones first: a rollout in both counts
    /// once, from `sessions`.
    pub fn roots(&self) -> [PathBuf; 2] {
        [self.dir.join(SESSIONS), self.dir.join(ARCHIVED)]
    }

    pub fn session_index(&self) -> PathBuf {
        self.dir.join(SESSION_INDEX)
    }

    pub fn display(&self) -> String {
        display_dir(&self.dir, self.home.as_deref())
    }

    /// A path in the user's notation, `~` for the home folder.
    pub fn display_path(&self, path: &str) -> String {
        display_dir(Path::new(path), self.home.as_deref())
    }

    /// An explicit `CODEX_HOME` that leads nowhere would otherwise read as
    /// no usage at all.
    pub fn problem(&self) -> Option<ReadError> {
        let value = self.from_env.as_ref()?;
        (!self.dir.is_dir()).then(|| ReadError {
            path: format!("{HOME_ENV}={value}"),
            message: "this folder does not exist".into(),
        })
    }
}

fn expand_home(entry: &str, home: Option<&Path>) -> PathBuf {
    match (entry.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(entry),
    }
}

/// How a rollout file is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Encoding {
    Plain,
    Zstd,
}

/// The rollout a file holds, as `rollout-….jsonl` whatever its encoding,
/// and how it is stored. `None` for files that are not rollouts.
pub fn rollout_key(path: &Path) -> Option<(String, Encoding)> {
    let name = path.file_name()?.to_str()?;
    if !name.starts_with("rollout-") {
        return None;
    }
    if name.ends_with(PLAIN) {
        Some((name.to_owned(), Encoding::Plain))
    } else {
        name.strip_suffix(".zst")
            .filter(|_| name.ends_with(COMPRESSED))
            .map(|plain| (plain.to_owned(), Encoding::Zstd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_comes_from_codex_home_or_defaults_to_dot_codex() {
        let home = PathBuf::from("/home/me");
        let found = CodexHome::resolve(None, Some(home.clone()));
        assert_eq!(found.dir, home.join(".codex"));
        assert!(
            found.problem().is_none(),
            "a missing default is not an error"
        );

        let found = CodexHome::resolve(Some(OsString::from("  ")), Some(home.clone()));
        assert_eq!(found.dir, home.join(".codex"), "blank means unset");

        let found = CodexHome::resolve(Some(OsString::from("~/work/codex")), Some(home.clone()));
        assert_eq!(found.dir, home.join("work/codex"));
        assert_eq!(
            found.roots(),
            [
                home.join("work/codex").join("sessions"),
                home.join("work/codex").join("archived_sessions"),
            ]
        );
        let problem = found.problem().expect("an explicit folder that is missing");
        assert_eq!(problem.path, "CODEX_HOME=~/work/codex");
    }

    #[test]
    fn rollouts_are_keyed_by_their_plain_name() {
        let key = |name: &str| rollout_key(Path::new(name));
        assert_eq!(
            key("rollout-2026-10-19T03-49-15-01a1.jsonl"),
            Some((
                "rollout-2026-10-19T03-49-15-01a1.jsonl".into(),
                Encoding::Plain
            ))
        );
        assert_eq!(
            key("rollout-2026-10-19T03-49-15-01a1.jsonl.zst"),
            Some((
                "rollout-2026-10-19T03-49-15-01a1.jsonl".into(),
                Encoding::Zstd
            ))
        );
        assert_eq!(key("session_index.jsonl"), None);
        assert_eq!(key("rollout-x.json"), None);
        assert_eq!(key("rollout-x.zst"), None);
    }
}
