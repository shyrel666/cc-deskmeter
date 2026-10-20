//! Where Claude Code keeps its transcripts:
//!
//! - `CLAUDE_CONFIG_DIR`, comma-separated, replaces the defaults entirely.
//!   An entry may name the config folder or its `projects` folder. A value
//!   that names an existing folder as a whole is that one folder, commas and
//!   all (`C:\Users\Smith, John\.claude`).
//! - Otherwise `$XDG_CONFIG_HOME/claude` (default `~/.config/claude`), then
//!   `~/.claude`; both are read when both exist.
//!
//! The app keeps running, so candidates that don't exist yet are kept and
//! picked up (scanned and watched) once they appear.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::types::ReadError;

pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
const PROJECTS: &str = "projects";
/// Claude Code's transcript retention when `cleanupPeriodDays` is unset.
const DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Debug, Clone, Default)]
pub struct Roots {
    /// `projects` folders to scan, in discovery order, without duplicates.
    pub dirs: Vec<PathBuf>,
    /// The raw `CLAUDE_CONFIG_DIR` value when it chose `dirs`.
    from_env: Option<String>,
    home: Option<PathBuf>,
}

impl From<PathBuf> for Roots {
    fn from(dir: PathBuf) -> Self {
        Self {
            dirs: vec![dir],
            from_env: None,
            home: None,
        }
    }
}

impl Roots {
    pub fn discover() -> Self {
        Self::resolve(
            std::env::var_os(CONFIG_DIR_ENV),
            std::env::var_os("XDG_CONFIG_HOME"),
            dirs::home_dir(),
        )
    }

    fn resolve(
        config_dirs: Option<OsString>,
        xdg: Option<OsString>,
        home: Option<PathBuf>,
    ) -> Self {
        // A blank value is treated as unset rather than as "no folders".
        let config_dirs = config_dirs
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.trim().is_empty());
        let candidates: Vec<PathBuf> = match &config_dirs {
            Some(value) => {
                let whole = value.trim();
                let entries: Vec<&str> = if expand_home(whole, home.as_deref()).is_dir() {
                    vec![whole]
                } else {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .collect()
                };
                entries
                    .into_iter()
                    .map(|entry| {
                        let dir = expand_home(entry, home.as_deref());
                        if dir.file_name().is_some_and(|name| name == PROJECTS) {
                            dir
                        } else {
                            dir.join(PROJECTS)
                        }
                    })
                    .collect()
            }
            None => home
                .as_deref()
                .map(|home| {
                    let config = xdg
                        .filter(|xdg| !xdg.is_empty())
                        .map_or_else(|| home.join(".config"), PathBuf::from);
                    vec![
                        config.join("claude").join(PROJECTS),
                        home.join(".claude").join(PROJECTS),
                    ]
                })
                .unwrap_or_default(),
        };

        // Different spellings of one folder (e.g. a symlink) are read once.
        let mut literal = HashSet::new();
        let mut canonical = HashSet::new();
        let dirs = candidates
            .into_iter()
            .filter(|dir| literal.insert(dir.clone()))
            .filter(|dir| {
                dir.canonicalize()
                    .map_or(true, |real| canonical.insert(real))
            })
            .collect();
        Self {
            dirs,
            from_env: config_dirs,
            home,
        }
    }

    /// Folders shown to the user: those that exist, or every candidate while
    /// none does, with the home folder written as `~`.
    pub fn display(&self) -> Vec<String> {
        let existing: Vec<&PathBuf> = self.dirs.iter().filter(|dir| dir.is_dir()).collect();
        let shown = if existing.is_empty() {
            self.dirs.iter().collect()
        } else {
            existing
        };
        shown
            .into_iter()
            .map(|dir| display_dir(dir, self.home.as_deref()))
            .collect()
    }

    /// Claude Code's `cleanupPeriodDays` from the first config folder's
    /// `settings.json` that sets it, else its default.
    pub fn retention_days(&self) -> u32 {
        self.dirs
            .iter()
            .filter_map(|dir| dir.parent())
            .find_map(|config| {
                let text = std::fs::read_to_string(config.join("settings.json")).ok()?;
                let settings: serde_json::Value = serde_json::from_str(&text).ok()?;
                let days = settings.get("cleanupPeriodDays")?.as_u64()?;
                Some(u32::try_from(days).unwrap_or(u32::MAX))
            })
            .unwrap_or(DEFAULT_RETENTION_DAYS)
    }

    /// An explicit `CLAUDE_CONFIG_DIR` that leads nowhere would otherwise read
    /// as zero usage.
    pub fn problem(&self) -> Option<ReadError> {
        let value = self.from_env.as_ref()?;
        if self.dirs.iter().any(|dir| dir.is_dir()) {
            return None;
        }
        Some(ReadError {
            path: format!("{CONFIG_DIR_ENV}={value}"),
            message: format!("none of these folders contains a `{PROJECTS}` folder"),
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

pub fn display_dir(dir: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| dir.strip_prefix(home).ok()) {
        Some(rest) => {
            let parts: Vec<_> = rest.iter().map(|part| part.to_string_lossy()).collect();
            format!("~/{}", parts.join("/"))
        }
        None => dir.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    struct TempHome(PathBuf);

    impl TempHome {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cc-deskmeter-paths-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn mkdir(&self, rel: &str) -> PathBuf {
            let dir = self.0.join(rel);
            fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn resolve(env: Option<&str>, xdg: Option<&Path>, home: &TempHome) -> Roots {
        Roots::resolve(
            env.map(OsString::from),
            xdg.map(|xdg| xdg.as_os_str().to_owned()),
            Some(home.0.clone()),
        )
    }

    #[test]
    fn defaults_list_xdg_then_legacy_folder() {
        let home = TempHome::new("defaults");
        let roots = resolve(None, None, &home);
        assert_eq!(
            roots.dirs,
            [
                home.0.join(".config").join("claude").join("projects"),
                home.0.join(".claude").join("projects"),
            ]
        );
        assert!(
            roots.problem().is_none(),
            "missing defaults are not an error"
        );
        // Nothing exists yet: show where we look.
        assert_eq!(
            roots.display(),
            ["~/.config/claude/projects", "~/.claude/projects"]
        );

        home.mkdir(".claude/projects");
        assert_eq!(roots.display(), ["~/.claude/projects"]);

        let xdg = home.mkdir("xdg");
        let roots = resolve(None, Some(&xdg), &home);
        assert_eq!(roots.dirs[0], xdg.join("claude").join("projects"));
    }

    #[test]
    fn retention_comes_from_the_first_settings_that_set_it() {
        let home = TempHome::new("retention");
        let roots = resolve(None, None, &home);
        assert_eq!(roots.retention_days(), 30, "Claude Code's default");

        home.mkdir(".config/claude");
        fs::write(
            home.0.join(".config/claude/settings.json"),
            r#"{"model":"opus"}"#,
        )
        .unwrap();
        home.mkdir(".claude");
        fs::write(
            home.0.join(".claude/settings.json"),
            r#"{"cleanupPeriodDays": 90}"#,
        )
        .unwrap();
        assert_eq!(
            roots.retention_days(),
            90,
            "a file without the key is skipped"
        );

        fs::write(home.0.join(".claude/settings.json"), "not json").unwrap();
        assert_eq!(roots.retention_days(), 30);
    }

    #[test]
    fn config_dir_env_replaces_defaults_and_accepts_lists() {
        let home = TempHome::new("env");
        home.mkdir(".claude/projects");
        let work = home.mkdir("work/projects");
        let value = format!(" ~/alt , {} ,, ~/alt ", work.display());
        let roots = resolve(Some(&value), None, &home);
        assert_eq!(
            roots.dirs,
            [home.0.join("alt").join("projects"), work.clone()],
            "trimmed, home-expanded, `projects` accepted, duplicates dropped"
        );
        assert!(roots.problem().is_none(), "one listed folder exists");

        let roots = resolve(Some("~/nowhere"), None, &home);
        assert_eq!(roots.dirs, [home.0.join("nowhere").join("projects")]);
        let problem = roots.problem().expect("explicit folder has no projects");
        assert_eq!(problem.path, "CLAUDE_CONFIG_DIR=~/nowhere");

        // Blank means unset.
        let roots = resolve(Some("  "), None, &home);
        assert_eq!(roots.dirs.len(), 2);
        assert!(roots.problem().is_none());
    }

    #[test]
    fn a_config_dir_with_commas_in_its_name_is_one_folder() {
        let home = TempHome::new("comma");
        let config = home.mkdir("Smith, John/.claude");
        home.mkdir("Smith, John/.claude/projects");
        let whole = config.display().to_string();
        for value in [whole.clone(), format!("  {whole} ")] {
            let roots = resolve(Some(&value), None, &home);
            assert_eq!(roots.dirs, [config.join("projects")], "{value}");
            assert!(roots.problem().is_none());
        }
        let roots = resolve(Some("~/Smith, John/.claude/projects"), None, &home);
        assert_eq!(roots.dirs, [config.join("projects")]);

        // A list is still a list, also when one of its folders exists.
        home.mkdir("a/projects");
        let roots = resolve(Some("~/a,~/Smith"), None, &home);
        assert_eq!(
            roots.dirs,
            [
                home.0.join("a").join("projects"),
                home.0.join("Smith").join("projects")
            ]
        );
    }

    #[test]
    fn explicit_projects_folder_can_appear_after_startup() {
        let home = TempHome::new("late-projects");
        let roots = resolve(Some("~/new/projects"), None, &home);
        let projects = home.0.join("new/projects");
        assert_eq!(roots.dirs, [projects.clone()]);
        assert!(roots.problem().is_some());

        let project = home.mkdir("new/projects/p");
        fs::write(
            project.join("s.jsonl"),
            concat!(
                r#"{"type":"assistant","timestamp":"2026-10-18T10:00:00Z","sessionId":"s","message":{"id":"m","model":"claude-opus-5-5","usage":{"input_tokens":100,"output_tokens":1}}}"#,
                "\n"
            ),
        ).unwrap();
        assert!(roots.problem().is_none());
        let mut store = super::super::store::UsageStore::new(roots);
        assert!(store.refresh());
        let data = store.snapshot(chrono::Utc::now());
        assert_eq!(data.file_count, 1);
        assert_eq!(data.total.total_tokens, 101);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_folders_are_read_once() {
        let home = TempHome::new("symlink");
        let real = home.mkdir(".claude");
        home.mkdir(".claude/projects");
        home.mkdir(".config");
        std::os::unix::fs::symlink(&real, home.0.join(".config").join("claude")).unwrap();
        let roots = resolve(None, None, &home);
        assert_eq!(roots.dirs.len(), 1);
    }
}
