#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt as _, symlink};

use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use soft_canonicalize::soft_canonicalize;
use tempfile::TempDir;

use super::locks::{Clock, SystemClock, lock_path_for, setup_locks};
use super::{
    Error, SAVED_SETUP_VERSION, SavedSetup, Target, forget_saved_setup, read_saved_setup,
    receipt_path_for, save_setup, settings_path_for, setup_profile_path,
};
use crate::agent_launch::codex_config_path;
use crate::claude_settings::paths::{
    claude_settings_path, claude_settings_paths, configure_state_path,
};
use crate::codex_settings::paths::codex_configure_state_path;

const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";

struct FakeEnvironment {
    variables: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl FakeEnvironment {
    fn new(home: Option<&Path>, variables: &[(&str, &str)]) -> Self {
        Self {
            variables: variables
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            home: home.map(Path::to_path_buf),
        }
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

struct StepClock {
    start: Instant,
    step: Duration,
    calls: Mutex<u32>,
}

impl StepClock {
    fn new(step: Duration) -> Self {
        Self {
            start: Instant::now(),
            step,
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> u32 {
        *self.calls.lock().expect("the call counter")
    }
}

impl Clock for StepClock {
    fn now(&self) -> Instant {
        let mut calls = self.calls.lock().expect("the call counter");
        let offset = self.step.checked_mul(*calls).expect("the clock offset");
        *calls = calls.checked_add(1).expect("the call count");
        drop(calls);
        self.start.checked_add(offset).expect("the clock instant")
    }
}

#[derive(Serialize)]
struct ProfileDocument<'text> {
    version: u8,
    target: &'text str,
    settings_path: &'text str,
    base_url: &'text str,
    api_key: &'text str,
    model: Option<&'text str>,
}

#[derive(Deserialize)]
struct WrittenProfile {
    version: u8,
    target: String,
    settings_path: String,
    base_url: String,
    api_key: String,
    model: Option<String>,
}

fn home_environment(home: &Path) -> FakeEnvironment {
    FakeEnvironment::new(Some(home), &[])
}

fn config_environment(home: &Path, config_dir: &str) -> FakeEnvironment {
    FakeEnvironment::new(Some(home), &[(CLAUDE_CONFIG_DIR, config_dir)])
}

fn settings_text(environment: &FakeEnvironment, target: Target) -> String {
    let path = settings_path_for(target, environment).expect("a settings path");
    soft_canonicalize(&path)
        .expect("a resolved settings path")
        .to_str()
        .expect("a UTF-8 settings path")
        .to_owned()
}

fn saved_profile(environment: &FakeEnvironment, target: Target, model: Option<&str>) -> SavedSetup {
    SavedSetup::new(
        target,
        settings_text(environment, target),
        "https://gateway.example.com".to_owned(),
        SecretString::from("sk-test"),
        model.map(str::to_owned),
    )
    .expect("a valid saved setup")
}

fn profile_document(target: &str, settings_path: &str, model: Option<&str>) -> String {
    serde_json::to_string(&ProfileDocument {
        version: SAVED_SETUP_VERSION,
        target,
        settings_path,
        base_url: "https://gateway.example.com",
        api_key: "sk-test",
        model,
    })
    .expect("a profile document")
}

fn write_profile(environment: &FakeEnvironment, target: Target, document: &str) -> PathBuf {
    let settings_path = settings_path_for(target, environment).expect("a settings path");
    let profile = setup_profile_path(target, &settings_path, environment).expect("a profile path");
    let parent = profile.parent().expect("a profile directory");
    fs::create_dir_all(parent).expect("the profile directory");
    fs::write(&profile, document).expect("the profile file");
    profile
}

fn expected_lock_path(target: Target, settings_path: &str, home: &Path) -> PathBuf {
    let digest = Sha256::digest(format!("{target}:{settings_path}"));
    home.join(".litellm")
        .join("setup-locks")
        .join(format!("{}.lock", hex::encode(digest)))
}

fn hold_lock(path: &Path) -> File {
    let parent = path.parent().expect("a lock directory");
    fs::create_dir_all(parent).expect("the lock directory");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("a lock file");
    file.try_lock().expect("the held lock");
    file
}

fn lock_state(path: &Path) -> Result<(), TryLockError> {
    let probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(TryLockError::Error)?;
    probe.try_lock()
}

fn saved_setup(base_url: &str, api_key: &str, model: Option<&str>) -> Result<SavedSetup, Error> {
    SavedSetup::new(
        Target::Claude,
        "/home/tester/.claude/settings.json".to_owned(),
        base_url.to_owned(),
        SecretString::from(api_key),
        model.map(str::to_owned),
    )
}

#[rstest]
#[case::trailing_slash("https://gateway.example.com/")]
#[case::version_suffix("https://gateway.example.com/v1")]
#[case::version_suffix_with_slash("https://gateway.example.com/v1/")]
fn base_url_rejects_unnormalized_value(#[case] base_url: &str) {
    assert!(matches!(
        saved_setup(base_url, "sk-test", None),
        Err(Error::BaseUrlNotNormalized)
    ));
}

#[rstest]
#[case::bare_host("gateway.example.com")]
#[case::wrong_scheme("ftp://gateway.example.com")]
#[case::empty_host("http://")]
#[case::query("https://gateway.example.com/?page=1")]
#[case::fragment("https://gateway.example.com/#top")]
#[case::credentials("https://user:pass@gateway.example.com")]
fn base_url_rejects_invalid_url(#[case] base_url: &str) {
    assert!(matches!(
        saved_setup(base_url, "sk-test", None),
        Err(Error::BaseUrlInvalid(_))
    ));
}

#[test]
fn api_key_rejects_empty() {
    assert!(matches!(
        saved_setup("https://gateway.example.com", "", None),
        Err(Error::ApiKeyInvalid)
    ));
}

#[rstest]
#[case::tab("sk\ttest")]
#[case::newline("sk\ntest")]
#[case::nul("sk\0test")]
#[case::delete("sk\u{7f}test")]
fn api_key_rejects_control_character(#[case] api_key: &str) {
    assert!(matches!(
        saved_setup("https://gateway.example.com", api_key, None),
        Err(Error::ApiKeyInvalid)
    ));
}

#[rstest]
#[case::unit_separator("model\u{1f}")]
#[case::delete("model\u{7f}")]
fn model_rejects_control_character(#[case] model: &str) {
    assert!(matches!(
        saved_setup("https://gateway.example.com", "sk-test", Some(model)),
        Err(Error::ModelInvalid)
    ));
}

#[test]
fn model_none_accepted() {
    let saved = saved_setup("https://gateway.example.com", "sk-test", None)
        .expect("a missing model is accepted");
    assert!(saved.model.is_none());
}

#[test]
fn model_rejects_empty() {
    assert!(matches!(
        saved_setup("https://gateway.example.com", "sk-test", Some("")),
        Err(Error::ModelInvalid)
    ));
}

#[test]
fn api_key_rejects_space() {
    assert!(matches!(
        saved_setup("https://gateway.example.com", "sk test", None),
        Err(Error::ApiKeyInvalid)
    ));
}

#[test]
fn model_allows_space() {
    let saved = saved_setup(
        "https://gateway.example.com",
        "sk-test",
        Some("claude sonnet"),
    )
    .expect("a model may hold a space");
    assert_eq!(saved.model.as_deref(), Some("claude sonnet"));
}

#[test]
fn settings_path_for_claude_uses_claude_settings_path() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    assert_eq!(
        settings_path_for(Target::Claude, &environment),
        claude_settings_path(&environment)
    );
}

#[test]
fn settings_path_for_codex_uses_codex_config_path() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    assert_eq!(
        settings_path_for(Target::Codex, &environment),
        codex_config_path(&environment)
    );
}

#[test]
fn receipt_path_for_claude_uses_configure_state_path() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let settings_path = settings_path_for(Target::Claude, &environment).expect("a settings path");
    let expected = configure_state_path(&settings_path, &claude_settings_paths(home.path()))
        .expect("a receipt path");
    assert_eq!(
        receipt_path_for(Target::Claude, &settings_path, &environment).expect("a receipt path"),
        expected
    );
}

#[test]
fn receipt_path_for_codex_uses_codex_configure_state_path() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let settings_path = settings_path_for(Target::Codex, &environment).expect("a settings path");
    assert_eq!(
        receipt_path_for(Target::Codex, &settings_path, &environment).expect("a receipt path"),
        codex_configure_state_path(&settings_path).expect("a receipt path")
    );
}

#[rstest]
#[case::claude(Target::Claude)]
#[case::codex(Target::Codex)]
fn setup_profile_path_renames_stem(#[case] target: Target) {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let settings_path = settings_path_for(target, &environment).expect("a settings path");
    let receipt = receipt_path_for(target, &settings_path, &environment).expect("a receipt path");
    let profile = setup_profile_path(target, &settings_path, &environment).expect("a profile path");
    assert_eq!(profile.parent(), receipt.parent());
    let stem = receipt.file_stem().expect("a receipt stem");
    let mut expected = stem.to_os_string();
    expected.push("_profile.json");
    assert_eq!(profile.file_name(), Some(expected.as_os_str()));
}

#[test]
fn read_saved_setup_returns_none_when_missing() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    assert!(
        read_saved_setup(Target::Claude, &environment)
            .expect("a missing profile is not an error")
            .is_none()
    );
}

#[test]
fn read_saved_setup_wraps_read_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None);
    let profile = write_profile(&environment, Target::Claude, &document);
    fs::remove_file(&profile).expect("the profile file");
    fs::create_dir_all(&profile).expect("a profile directory");
    let Err(error) = read_saved_setup(Target::Claude, &environment) else {
        panic!("a read failure is reported");
    };
    assert_eq!(
        error.to_string(),
        format!(
            "Could not read saved claude setup at {}; no settings were changed",
            profile.display()
        )
    );
    assert!(matches!(error, Error::Read(Target::Claude, _, _)));
}

#[test]
fn read_saved_setup_rejects_target_mismatch() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("codex", &settings_text(&environment, Target::Claude), None);
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::Invalid(Target::Claude, _))
    ));
}

#[test]
fn read_saved_setup_rejects_settings_path_mismatch() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let other = home.path().join(".claude").join("other.json");
    let document = profile_document(
        "claude",
        other.to_str().expect("a UTF-8 settings path"),
        None,
    );
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::Invalid(Target::Claude, _))
    ));
}

#[test]
fn read_saved_setup_rejects_codex_without_model() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("codex", &settings_text(&environment, Target::Codex), None);
    write_profile(&environment, Target::Codex, &document);
    assert!(matches!(
        read_saved_setup(Target::Codex, &environment),
        Err(Error::Invalid(Target::Codex, _))
    ));
}

#[test]
fn read_saved_setup_accepts_claude_without_model() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None);
    write_profile(&environment, Target::Claude, &document);
    let saved = read_saved_setup(Target::Claude, &environment)
        .expect("a valid profile")
        .expect("a saved setup");
    assert_eq!(saved.target, Target::Claude);
    assert!(saved.model.is_none());
}

#[test]
fn read_saved_setup_rejects_unknown_version() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let current = format!("\"version\":{SAVED_SETUP_VERSION}");
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None)
        .replace(&current, "\"version\":2");
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::Invalid(Target::Claude, _))
    ));
}

#[test]
fn read_saved_setup_rejects_unknown_key() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None)
        .replacen('}', ",\"extra\":1}", 1);
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::InvalidJson(Target::Claude, _, _))
    ));
}

#[test]
fn read_saved_setup_defaults_missing_version() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let current = format!("\"version\":{SAVED_SETUP_VERSION},");
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None)
        .replace(&current, "");
    write_profile(&environment, Target::Claude, &document);
    let saved = read_saved_setup(Target::Claude, &environment)
        .expect("a missing version defaults to one")
        .expect("a saved setup");
    assert_eq!(saved.target, Target::Claude);
}

#[test]
fn read_saved_setup_rejects_missing_model_key() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None)
        .replace(",\"model\":null", "");
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::Invalid(Target::Claude, _))
    ));
}

#[test]
fn read_saved_setup_wraps_validator_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None)
        .replace(
            "\"base_url\":\"https://gateway.example.com\"",
            "\"base_url\":\"https://gateway.example.com/\"",
        );
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::InvalidValues(Target::Claude, _, _))
    ));
}

#[cfg(unix)]
#[test]
fn read_saved_setup_wraps_resolution_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let first = home.path().join("first");
    let second = home.path().join("second");
    symlink(&second, &first).expect("the first link");
    symlink(&first, &second).expect("the second link");
    let settings = first.join("settings.json");
    let document = profile_document(
        "claude",
        settings.to_str().expect("a UTF-8 settings path"),
        None,
    );
    write_profile(&environment, Target::Claude, &document);
    assert!(matches!(
        read_saved_setup(Target::Claude, &environment),
        Err(Error::InvalidPath(Target::Claude, _, _))
    ));
}

#[test]
fn read_saved_setup_error_names_unconfigure_forget() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("codex", &settings_text(&environment, Target::Claude), None);
    let profile = write_profile(&environment, Target::Claude, &document);
    let Err(error) = read_saved_setup(Target::Claude, &environment) else {
        panic!("a mismatched target is rejected");
    };
    assert_eq!(
        error.to_string(),
        format!(
            "Saved claude setup at {} is invalid or unsupported. Run `lite unconfigure claude --forget` to discard it; no settings were changed",
            profile.display()
        )
    );
}

#[tokio::test]
async fn setup_locks_acquires_claude_before_codex() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let codex = lock_path_for(Target::Codex, &environment).expect("a lock path");
    let _held = hold_lock(&codex);
    let clock = StepClock::new(Duration::from_secs(2));
    let result = setup_locks(&[Target::Codex, Target::Claude], &environment, &clock).await;
    assert!(matches!(result, Err(Error::Locked)));
    let claude = lock_path_for(Target::Claude, &environment).expect("a lock path");
    assert!(
        claude.exists(),
        "Claude's lock is taken before Codex blocks"
    );
}

#[tokio::test]
async fn setup_locks_dedupes_repeated_target() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let clock = StepClock::new(Duration::from_secs(2));
    let guard = setup_locks(&[Target::Claude, Target::Claude], &environment, &clock)
        .await
        .expect("the lock is acquired once");
    let claude = lock_path_for(Target::Claude, &environment).expect("a lock path");
    assert!(matches!(lock_state(&claude), Err(TryLockError::WouldBlock)));
    drop(guard);
}

#[cfg(unix)]
#[tokio::test]
async fn setup_locks_digest_keyed_by_resolved_settings_path() {
    let home = TempDir::new().expect("a home");
    fs::create_dir_all(home.path().join("real")).expect("the real config directory");
    let link = home.path().join("link");
    symlink(home.path().join("real"), &link).expect("the config symlink");
    let config_dir = link.to_str().expect("a UTF-8 config path");
    let environment = config_environment(home.path(), config_dir);
    let raw = settings_path_for(Target::Claude, &environment)
        .expect("a settings path")
        .to_str()
        .expect("a UTF-8 settings path")
        .to_owned();
    let resolved = settings_text(&environment, Target::Claude);
    let guard = setup_locks(&[Target::Claude], &environment, &SystemClock)
        .await
        .expect("the lock");
    let expected = expected_lock_path(Target::Claude, &resolved, home.path());
    assert!(expected.exists());
    assert!(!expected_lock_path(Target::Claude, &raw, home.path()).exists());
    assert!(matches!(
        lock_state(&expected),
        Err(TryLockError::WouldBlock)
    ));
    drop(guard);
}

#[cfg(unix)]
#[tokio::test]
async fn setup_locks_lock_file_mode_owner_only() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let lock = lock_path_for(Target::Claude, &environment).expect("a lock path");
    let guard = setup_locks(&[Target::Claude], &environment, &SystemClock)
        .await
        .expect("the lock");
    let mode = fs::metadata(&lock)
        .expect("the lock file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    drop(guard);
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).expect("a wide lock file");
    let _guard = setup_locks(&[Target::Claude], &environment, &SystemClock)
        .await
        .expect("the lock");
    let tightened = fs::metadata(&lock)
        .expect("the lock file")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(tightened, 0o600);
}

#[tokio::test]
async fn setup_locks_times_out_after_ten_seconds() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let claude = lock_path_for(Target::Claude, &environment).expect("a lock path");
    let _held = hold_lock(&claude);
    let clock = StepClock::new(Duration::from_secs(1));
    let result = setup_locks(&[Target::Claude], &environment, &clock).await;
    assert!(matches!(result, Err(Error::Locked)));
    assert_eq!(
        clock.calls(),
        11,
        "the tenth second is the first to lock out"
    );
}

#[tokio::test]
async fn setup_locks_releases_acquired_locks_on_timeout() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let codex = lock_path_for(Target::Codex, &environment).expect("a lock path");
    let _held = hold_lock(&codex);
    let clock = StepClock::new(Duration::from_secs(2));
    let result = setup_locks(&[Target::Claude, Target::Codex], &environment, &clock).await;
    assert!(matches!(result, Err(Error::Locked)));
    let claude = lock_path_for(Target::Claude, &environment).expect("a lock path");
    assert!(
        lock_state(&claude).is_ok(),
        "the Claude lock is released on failure"
    );
}

#[tokio::test]
async fn setup_locks_wraps_directory_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    fs::write(home.path().join(".litellm"), "not a directory").expect("a blocking file");
    let result = setup_locks(&[Target::Claude], &environment, &SystemClock).await;
    let Err(error) = result else {
        panic!("a lock directory failure is reported");
    };
    assert_eq!(
        error.to_string(),
        "Could not lock agent setup; retry when other configure commands finish"
    );
    let Error::LockPrepare(lock, _) = error else {
        panic!("the directory failure is typed");
    };
    assert_eq!(
        lock,
        lock_path_for(Target::Claude, &environment).expect("a lock path")
    );
}

#[tokio::test]
async fn save_setup_creates_parent_directory() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let saved = saved_profile(&environment, Target::Claude, None);
    save_setup(&saved, &environment)
        .await
        .expect("the saved setup");
    let settings_path = settings_path_for(Target::Claude, &environment).expect("a settings path");
    let profile =
        setup_profile_path(Target::Claude, &settings_path, &environment).expect("a profile path");
    let parent = profile.parent().expect("a profile directory");
    assert!(parent.is_dir());
    assert!(profile.is_file());
    #[cfg(unix)]
    {
        let mode = fs::metadata(parent)
            .expect("the profile directory")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
}

#[tokio::test]
async fn save_setup_writes_private_json() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let saved = saved_profile(&environment, Target::Claude, Some("claude-sonnet-5"));
    save_setup(&saved, &environment)
        .await
        .expect("the saved setup");
    let settings_path = settings_path_for(Target::Claude, &environment).expect("a settings path");
    let profile =
        setup_profile_path(Target::Claude, &settings_path, &environment).expect("a profile path");
    let bytes = fs::read(&profile).expect("the profile file");
    let written: WrittenProfile = serde_json::from_slice(&bytes).expect("the stored setup");
    assert_eq!(written.version, SAVED_SETUP_VERSION);
    assert_eq!(written.target, "claude");
    assert_eq!(written.settings_path, saved.settings_path);
    assert_eq!(written.base_url, "https://gateway.example.com");
    assert_eq!(written.api_key, "sk-test");
    assert_eq!(written.model.as_deref(), Some("claude-sonnet-5"));
    #[cfg(unix)]
    {
        let mode = fs::metadata(&profile)
            .expect("the profile file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[tokio::test]
async fn save_setup_wraps_directory_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let saved = saved_profile(&environment, Target::Claude, None);
    fs::write(home.path().join(".litellm"), "not a directory").expect("a blocking file");
    let error = save_setup(&saved, &environment)
        .await
        .expect_err("a directory failure");
    assert!(matches!(error, Error::Save(Target::Claude, _)));
    assert_eq!(
        error.to_string(),
        "Could not save claude setup; no claude settings were changed"
    );
}

#[tokio::test]
async fn save_setup_wraps_write_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let saved = saved_profile(&environment, Target::Claude, None);
    let settings_path = settings_path_for(Target::Claude, &environment).expect("a settings path");
    let profile =
        setup_profile_path(Target::Claude, &settings_path, &environment).expect("a profile path");
    fs::create_dir_all(&profile).expect("a blocking directory");
    let error = save_setup(&saved, &environment)
        .await
        .expect_err("a write failure");
    assert!(matches!(error, Error::Save(Target::Claude, _)));
    assert_eq!(
        error.to_string(),
        "Could not save claude setup; no claude settings were changed"
    );
}

#[test]
fn forget_saved_setup_tolerates_missing_file() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    forget_saved_setup(Target::Claude, &environment).expect("a missing profile is not an error");
}

#[test]
fn forget_saved_setup_wraps_removal_failure() {
    let home = TempDir::new().expect("a home");
    let environment = home_environment(home.path());
    let document = profile_document("claude", &settings_text(&environment, Target::Claude), None);
    let profile = write_profile(&environment, Target::Claude, &document);
    fs::remove_file(&profile).expect("the profile file");
    fs::create_dir_all(&profile).expect("a profile directory");
    let error = forget_saved_setup(Target::Claude, &environment).expect_err("a removal failure");
    assert_eq!(
        error.to_string(),
        format!(
            "Could not remove saved claude setup at {}",
            profile.display()
        )
    );
    assert!(matches!(error, Error::Forget(Target::Claude, _, _)));
}
