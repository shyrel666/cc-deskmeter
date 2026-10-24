//! Who is signed in to Codex, from `$CODEX_HOME/auth.json`: its
//! `tokens.account_id`, to tell accounts apart, and the email in its
//! `id_token` claims, to name them. Nothing else of the file is kept,
//! logged or shown, and no error repeats its content. `auth.json.bak`, the
//! previous sign-in an account switcher set aside, names that account too.
//!
//! The file names the account signed in now, and its last write says since
//! when: Codex and account switchers rewrite it at every sign-in (a token
//! refresh rewrites it too, for the same account). It keeps no history, so
//! each sign-in seen is kept in `codex-signins.json` beside the app's other
//! files, as the span from that write to the last time the file was seen
//! unchanged. A request sent within a span went out under its account,
//! whichever thread it continued: what the rollouts cannot tell once the
//! user switches accounts or the limits reset early (see `accounts`). Each
//! account's email is kept there too, so it stays named once signed out.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::usage::write_atomic;

const AUTH_FILE: &str = "auth.json";
const BACKUP_FILE: &str = "auth.json.bak";
const FILE: &str = "codex-signins.json";
const VERSION: u32 = 1;
/// Sign-ins kept, the latest: years of switching every day.
const KEEP: usize = 2000;
/// How far the current sign-in's end may run ahead of the saved one.
const SAVE_EVERY: TimeDelta = TimeDelta::minutes(10);

/// One sign-in as the app saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignIn {
    pub account: Arc<str>,
    /// When `auth.json` was written for it, or when the app last saw the
    /// sign-in before it if the file claims to be older (a copy that kept
    /// its time).
    pub from: DateTime<Utc>,
    /// The last time the file was seen unchanged.
    pub to: DateTime<Utc>,
}

impl SignIn {
    /// Whether a request sent no earlier than `asked`, and logged at `at`,
    /// went out under this sign-in.
    pub fn covers(&self, asked: DateTime<Utc>, at: DateTime<Utc>) -> bool {
        self.from <= asked && at <= self.to
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    signins: Vec<SignIn>,
    /// By account id.
    #[serde(default)]
    emails: BTreeMap<String, String>,
}

/// What is read of `auth.json`: the account id and its `id_token`, whose
/// claims name it.
#[derive(Deserialize)]
struct Auth {
    tokens: Option<Tokens>,
}

#[derive(Deserialize)]
struct Tokens {
    account_id: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/auth")]
    auth: Option<ClaimsAuth>,
}

#[derive(Deserialize)]
struct ClaimsAuth {
    chatgpt_account_id: Option<String>,
}

/// An account as an auth file names it.
struct Named {
    account: Arc<str>,
    email: Option<String>,
}

#[derive(Debug, Default)]
pub struct SignIns {
    auth: PathBuf,
    backup: PathBuf,
    /// Where the sign-ins are kept; `None` keeps them in memory only.
    file: Option<PathBuf>,
    /// By `from`.
    spans: Vec<SignIn>,
    /// The last of `spans` is who is signed in now.
    open: bool,
    /// `auth.json`'s last write as of its last complete read.
    written: Option<SystemTime>,
    /// `auth.json.bak`'s, likewise.
    backup_written: Option<SystemTime>,
    /// Each account's email, by account id, as last read.
    emails: BTreeMap<String, String>,
    /// The `to` of the current sign-in as last saved.
    saved: Option<DateTime<Utc>>,
}

impl SignIns {
    pub fn new(home: &Path) -> Self {
        Self {
            auth: home.join(AUTH_FILE),
            backup: home.join(BACKUP_FILE),
            ..Self::default()
        }
    }

    /// Keeps the sign-ins in `dir`, after those kept there before.
    pub fn open(&mut self, dir: &Path) {
        let file = dir.join(FILE);
        let kept = fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| saved.version == VERSION);
        let (kept, mut emails) =
            kept.map_or_else(Default::default, |saved| (saved.signins, saved.emails));
        // Those read since start-up are the latest.
        emails.append(&mut self.emails);
        self.emails = emails;
        let seen = std::mem::take(&mut self.spans);
        for span in kept.into_iter().chain(seen) {
            match self
                .spans
                .iter_mut()
                .find(|kept| kept.account == span.account && kept.from == span.from)
            {
                Some(kept) => kept.to = kept.to.max(span.to),
                None => self.spans.push(span),
            }
        }
        self.spans.sort_by_key(|span| span.from);
        self.file = Some(file);
        self.saved = None;
        self.save();
    }

    /// Every sign-in seen, by time.
    pub fn spans(&self) -> &[SignIn] {
        &self.spans
    }

    /// Who is signed in now, and since when; `None` while no one is (or
    /// `auth.json` names no account: an API key).
    pub fn current(&self) -> Option<&SignIn> {
        self.spans.last().filter(|_| self.open)
    }

    /// The account's email, once an auth file named it.
    pub fn email(&self, account: &str) -> Option<&str> {
        self.emails.get(account).map(String::as_str)
    }

    /// Reads the auth files again once they changed, and stretches the
    /// current sign-in to `now`. Returns whether who is signed in, or an
    /// account's email, changed.
    pub fn observe(&mut self, now: DateTime<Utc>) -> bool {
        let renamed = self.observe_backup();
        let changed = self.observe_auth(now) || renamed;
        if renamed {
            self.save();
        }
        changed
    }

    /// The previous sign-in's account, set aside by a switcher: its email.
    fn observe_backup(&mut self) -> bool {
        let written = fs::metadata(&self.backup)
            .and_then(|meta| meta.modified())
            .ok();
        if written.is_none() || written == self.backup_written {
            return false;
        }
        let Ok(named) = read_named(&self.backup) else {
            return false;
        };
        self.backup_written = written;
        named.is_some_and(|named| self.name(&named))
    }

    /// Keeps `named`'s email; returns whether it is new.
    fn name(&mut self, named: &Named) -> bool {
        let Some(email) = &named.email else {
            return false;
        };
        let known = self.emails.insert(named.account.to_string(), email.clone());
        known.as_ref() != Some(email)
    }

    fn observe_auth(&mut self, now: DateTime<Utc>) -> bool {
        let written = fs::metadata(&self.auth)
            .and_then(|meta| meta.modified())
            .ok();
        let changed = if written.is_some() && written == self.written {
            false
        } else {
            let Some(written) = written else {
                self.written = None;
                return self.close();
            };
            match read_named(&self.auth) {
                // Caught mid-write, perhaps: read again next time.
                Err(()) => return false,
                Ok(None) => {
                    self.written = Some(written);
                    return self.close();
                }
                Ok(Some(named)) => {
                    self.written = Some(written);
                    let renamed = self.name(&named);
                    self.sign_in(named.account, DateTime::<Utc>::from(written)) || renamed
                }
            }
        };
        if let Some(current) = self.spans.last_mut().filter(|_| self.open) {
            current.to = current.to.max(now);
        }
        if changed
            || self.current().is_some_and(|current| {
                self.saved
                    .is_none_or(|saved| current.to - saved >= SAVE_EVERY)
            })
        {
            self.save();
        }
        changed
    }

    /// `account` is signed in since the file's write at `written`.
    fn sign_in(&mut self, account: Arc<str>, written: DateTime<Utc>) -> bool {
        let was_open = self.open;
        self.open = true;
        let Some(last) = self.spans.last_mut() else {
            self.spans.push(SignIn {
                account,
                from: written,
                to: written,
            });
            return true;
        };
        // Written before the app last saw this account signed in: the same sign-in.
        if last.account == account && written <= last.to {
            return !was_open;
        }
        let from = written.max(last.to);
        self.spans.push(SignIn {
            account,
            from,
            to: from,
        });
        if self.spans.len() > KEEP {
            self.spans.drain(..self.spans.len() - KEEP);
        }
        true
    }

    /// No one is signed in now; returns whether someone was.
    fn close(&mut self) -> bool {
        let was_open = self.open;
        self.open = false;
        if was_open {
            self.save();
        }
        was_open
    }

    fn save(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let saved = Saved {
            version: VERSION,
            signins: self.spans.clone(),
            emails: self.emails.clone(),
        };
        let written = serde_json::to_vec(&saved)
            .map_err(|err| err.to_string())
            .and_then(|bytes| write_atomic(file, &bytes).map_err(|err| err.to_string()));
        match written {
            Ok(()) => self.saved = self.current().map(|current| current.to),
            Err(err) => eprintln!("cc-deskmeter: failed to save {}: {err}", file.display()),
        }
    }
}

#[cfg(test)]
impl SignIns {
    /// Sign-ins as if seen, the last one signed in now.
    pub fn seen(spans: Vec<SignIn>) -> Self {
        Self {
            open: !spans.is_empty(),
            spans,
            ..Self::default()
        }
    }
}

/// The account an auth file names: `Err` when it cannot be read or parsed,
/// `Ok(None)` when it names none.
fn read_named(path: &Path) -> Result<Option<Named>, ()> {
    let bytes = fs::read(path).map_err(|_| ())?;
    let auth: Auth = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let Some(tokens) = auth.tokens else {
        return Ok(None);
    };
    let Some(account) = tokens
        .account_id
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
    else {
        return Ok(None);
    };
    let email = tokens.id_token.and_then(|token| email_of(&token, &account));
    Ok(Some(Named {
        account: Arc::from(account),
        email,
    }))
}

/// The email in an `id_token`'s claims, if they are `account`'s own.
fn email_of(id_token: &str, account: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let json = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: Claims = serde_json::from_slice(&json).ok()?;
    let own = claims
        .auth
        .and_then(|auth| auth.chatgpt_account_id)
        .is_some_and(|id| id == account);
    claims
        .email
        .map(|email| email.trim().to_owned())
        .filter(|email| own && email.contains('@'))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cc-deskmeter-signin-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_792_000_000 + minute * 60, 0).unwrap()
    }

    /// Writes `auth.json` for `account`, last written at `minute`.
    fn sign_in(dir: &Path, account: &str, minute: i64) {
        write_auth(&dir.join(AUTH_FILE), account, account, minute);
    }

    /// An auth file for `account` whose `id_token` claims are `owner`'s,
    /// with the email `{owner}@example.com`.
    fn write_auth(path: &Path, account: &str, owner: &str, minute: i64) {
        let claims = format!(
            r#"{{"email":"{owner}@example.com","name":"Someone","https://api.openai.com/auth":{{"chatgpt_account_id":"{owner}","chatgpt_plan_type":"plus"}}}}"#
        );
        let id_token = format!("e30.{}.c2ln", URL_SAFE_NO_PAD.encode(claims));
        let body = format!(
            r#"{{"OPENAI_API_KEY":null,"auth_mode":"chatgpt","tokens":{{"id_token":"{id_token}","access_token":"secret","refresh_token":"secret","account_id":"{account}"}},"last_refresh":"2026-10-23T00:00:00Z"}}"#
        );
        fs::write(path, body).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::from(at(minute))).unwrap();
    }

    fn spans(signins: &SignIns) -> Vec<(&str, i64, i64)> {
        let minute = |t: DateTime<Utc>| (t - at(0)).num_minutes();
        signins
            .spans()
            .iter()
            .map(|span| (&*span.account, minute(span.from), minute(span.to)))
            .collect()
    }

    #[test]
    fn each_sign_in_spans_from_its_write_to_the_last_look() {
        let home = TempDir::new("spans");
        let mut signins = SignIns::new(&home.0);
        assert!(!signins.observe(at(0)), "no auth.json: no one");
        assert!(signins.current().is_none());

        sign_in(&home.0, "a", 10);
        assert!(signins.observe(at(20)));
        assert!(!signins.observe(at(30)));
        assert_eq!(spans(&signins), [("a", 10, 30)]);
        assert_eq!(&*signins.current().unwrap().account, "a");

        // A token refresh: the same account, written again.
        sign_in(&home.0, "a", 40);
        assert!(signins.observe(at(45)));
        // A switch, whose file kept an older time: it began after the last look.
        sign_in(&home.0, "b", 5);
        assert!(signins.observe(at(60)));
        assert_eq!(
            spans(&signins),
            [("a", 10, 30), ("a", 40, 45), ("b", 45, 60)]
        );

        // Signed out.
        fs::remove_file(home.0.join(AUTH_FILE)).unwrap();
        assert!(signins.observe(at(70)));
        assert!(signins.current().is_none());
        assert_eq!(spans(&signins).last(), Some(&("b", 45, 60)));
    }

    #[test]
    fn a_file_without_an_account_or_half_written_names_no_one() {
        let home = TempDir::new("partial");
        let mut signins = SignIns::new(&home.0);
        fs::write(
            home.0.join(AUTH_FILE),
            r#"{"OPENAI_API_KEY":"sk-secret","tokens":null}"#,
        )
        .unwrap();
        assert!(!signins.observe(at(0)));
        assert!(signins.current().is_none());
        fs::write(home.0.join(AUTH_FILE), r#"{"tokens":{"account_id":"a""#).unwrap();
        assert!(!signins.observe(at(1)));
        assert!(signins.current().is_none());
        // Read again once complete, though its time may not have moved.
        sign_in(&home.0, "a", 2);
        assert!(signins.observe(at(3)));
        assert_eq!(spans(&signins), [("a", 2, 3)]);
    }

    #[test]
    fn auth_files_name_their_accounts_by_email() {
        let home = TempDir::new("emails");
        let mut signins = SignIns::new(&home.0);
        sign_in(&home.0, "a", 10);
        assert!(signins.observe(at(11)));
        assert_eq!(signins.email("a"), Some("a@example.com"));
        // A switcher set a's file aside: b is named, a stays named.
        fs::rename(home.0.join(AUTH_FILE), home.0.join(BACKUP_FILE)).unwrap();
        sign_in(&home.0, "b", 20);
        assert!(signins.observe(at(21)));
        assert_eq!(signins.email("a"), Some("a@example.com"));
        assert_eq!(signins.email("b"), Some("b@example.com"));
        // Claims of another account name no one.
        write_auth(&home.0.join(AUTH_FILE), "c", "d", 30);
        signins.observe(at(31));
        assert_eq!(signins.email("c"), None);
        assert_eq!(signins.email("d"), None);
        // The set-aside copy alone names its account.
        let other = TempDir::new("emails-backup");
        write_auth(&other.0.join(BACKUP_FILE), "e", "e", 40);
        let mut backup_only = SignIns::new(&other.0);
        assert!(backup_only.observe(at(41)));
        assert!(backup_only.current().is_none());
        assert_eq!(backup_only.email("e"), Some("e@example.com"));
    }

    #[test]
    fn sign_ins_are_kept_and_carry_on_across_restarts() {
        let home = TempDir::new("home");
        let config = TempDir::new("config");
        sign_in(&home.0, "a", 10);
        let mut first = SignIns::new(&home.0);
        first.open(&config.0);
        first.observe(at(20));
        // Saved at least every ten minutes while it lasts.
        first.observe(at(26));
        first.observe(at(31));
        let kept = fs::read_to_string(config.0.join(FILE)).unwrap();
        assert!(
            !kept.contains("secret") && !kept.contains("e30"),
            "no token is kept"
        );
        assert!(!kept.contains("Someone"), "nor any claim but the email");

        // The app was closed from minute 31; the file did not change.
        let mut second = SignIns::new(&home.0);
        second.open(&config.0);
        assert_eq!(spans(&second), [("a", 10, 31)]);
        assert_eq!(
            second.email("a"),
            Some("a@example.com"),
            "named before any look"
        );
        second.observe(at(500));
        assert_eq!(spans(&second), [("a", 10, 500)]);

        // Switched while the app was closed: from the write on.
        drop(second);
        std::thread::sleep(Duration::from_millis(5));
        sign_in(&home.0, "b", 700);
        let mut third = SignIns::new(&home.0);
        third.open(&config.0);
        third.observe(at(800));
        assert_eq!(spans(&third), [("a", 10, 500), ("b", 700, 800)]);
    }
}
