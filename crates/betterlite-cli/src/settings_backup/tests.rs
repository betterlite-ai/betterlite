#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::TempDir;

use crate::claude_settings::SettingsDocument;
use crate::claude_settings::paths::claude_settings_paths;
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::ManagedTokenFile;
use crate::keyring::KeyringUnusable;

use super::down::{DOWN_ABOUT, DOWN_LONG_ABOUT, down};
use super::error::BackupError;
use super::probes::{
    KeyResolvedRequest, key_resolved_on_the_way_in, stored_login_is_pkce, usable_login,
};
use super::read::read_backup;
use super::record::BackupRecord;
use super::restore::restore_claude_settings;
use super::secure::secure_create;
use super::write::write_backup;

mod support;

use support::{
    FakeHttp, FakeVault, FixedWallClock, OTHER_SERVER, Reply, SERVER, TOKEN_ENDPOINT, exposed,
    record, secret, seed, token_file, token_response,
};

fn text(bytes: &[u8]) -> &str {
    str::from_utf8(bytes).expect("the captured output is UTF-8")
}

fn document() -> SettingsDocument {
    let mut settings = SettingsDocument::new();
    settings.insert("model".to_owned(), JsonValue::String("opus".to_owned()));
    settings
}

fn record_with(content: Option<SettingsDocument>, existed: bool) -> BackupRecord {
    BackupRecord { existed, content }
}

struct TestEnvironment {
    home: Option<PathBuf>,
    variables: BTreeMap<String, String>,
}

impl TestEnvironment {
    fn at(home: &Path) -> Self {
        Self {
            home: Some(home.to_path_buf()),
            variables: BTreeMap::new(),
        }
    }

    fn with_variables(home: &Path, variables: &[(&str, &str)]) -> Self {
        Self {
            home: Some(home.to_path_buf()),
            variables: variables
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn homeless() -> Self {
        Self {
            home: None,
            variables: BTreeMap::new(),
        }
    }
}

impl EnvironmentLookup for TestEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[test]
fn backup_secure_create_makes_a_private_file() {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    let file = secure_create(&path).expect("creates the file");
    drop(file);
    let mode = fs::metadata(&path)
        .expect("the file metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(fs::read(&path).expect("reads the file"), Vec::<u8>::new());
}

#[test]
fn backup_secure_create_tightens_an_existing_file() {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    fs::write(&path, b"old").expect("existing file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("wide mode");
    let file = secure_create(&path).expect("opens the file");
    drop(file);
    let mode = fs::metadata(&path)
        .expect("the file metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(fs::read(&path).expect("reads the file"), Vec::<u8>::new());
}

#[test]
fn backup_write_and_read_round_trip() {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("nested").join("backup.json");
    let files = PrivateFiles::default();
    let written = record_with(Some(document()), true);
    write_backup(&written, &path, &files).expect("writes the backup");
    let read = read_backup(&path)
        .expect("reads the backup")
        .expect("a record");
    assert!(read.existed);
    assert_eq!(read.content, Some(document()));
    let file_mode = fs::metadata(&path)
        .expect("the file metadata")
        .permissions()
        .mode();
    assert_eq!(file_mode & 0o777, 0o600);
    let parent_mode = fs::metadata(path.parent().expect("a parent"))
        .expect("the directory metadata")
        .permissions()
        .mode();
    assert_eq!(parent_mode & 0o777, 0o700);
}

#[test]
fn backup_read_missing_file_is_none() {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    assert!(read_backup(&path).expect("no backup").is_none());
}

#[test]
fn backup_read_names_the_path_on_a_read_failure() {
    let directory = TempDir::new().expect("directory");
    let error = read_backup(directory.path()).expect_err("refuses a directory");
    assert!(matches!(&error, BackupError::Read { path, .. } if path.as_path() == directory.path()));
    assert!(
        error
            .to_string()
            .contains(&directory.path().display().to_string())
    );
}

#[test]
fn backup_read_rejects_invalid_json() {
    let directory = TempDir::new().expect("directory");
    let backup = directory.path().join("backup.json");
    fs::write(&backup, b"not json").expect("writes the backup");
    let error = read_backup(&backup).expect_err("rejects");
    assert!(matches!(&error, BackupError::Invalid { path, .. } if path == &backup));
    assert!(error.to_string().contains(&backup.display().to_string()));
    assert!(error.to_string().contains("cannot restore from it safely"));
}

#[rstest]
#[case::missing_existed(r#"{"content":{}}"#)]
#[case::missing_content(r#"{"existed":true}"#)]
fn backup_read_rejects_a_missing_field(#[case] body: &str) {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    fs::write(&path, body).expect("writes the backup");
    let error = read_backup(&path).expect_err("rejects");
    assert!(matches!(error, BackupError::Invalid { .. }));
}

#[rstest]
#[case::existed_not_a_bool(r#"{"existed":"yes","content":{}}"#)]
#[case::content_an_array(r#"{"existed":true,"content":[]}"#)]
#[case::content_a_string(r#"{"existed":true,"content":"x"}"#)]
fn backup_read_rejects_a_wrong_type(#[case] body: &str) {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    fs::write(&path, body).expect("writes the backup");
    let error = read_backup(&path).expect_err("rejects");
    assert!(matches!(error, BackupError::Invalid { .. }));
}

#[test]
fn backup_read_ignores_other_keys() {
    let directory = TempDir::new().expect("directory");
    let path = directory.path().join("backup.json");
    fs::write(&path, br#"{"existed":false,"content":null,"extra":1}"#).expect("writes the backup");
    let read = read_backup(&path)
        .expect("reads the backup")
        .expect("a record");
    assert!(!read.existed);
    assert!(read.content.is_none());
}

#[test]
fn backup_write_reports_a_directory_it_cannot_prepare() {
    let directory = TempDir::new().expect("directory");
    let blocking = directory.path().join("blocked");
    fs::write(&blocking, b"not a directory").expect("blocking file");
    let files = PrivateFiles::default();
    let error = write_backup(
        &record_with(None, false),
        &blocking.join("backup.json"),
        &files,
    )
    .expect_err("refuses");
    assert!(matches!(error, BackupError::Directory { .. }));
}

fn restore_fixture() -> (TempDir, PrivateFiles) {
    (TempDir::new().expect("directory"), PrivateFiles::default())
}

#[test]
fn restore_writes_the_saved_content_back() {
    let (directory, files) = restore_fixture();
    let settings = directory.path().join(".claude").join("settings.json");
    let backup = directory.path().join(".litellm").join("backup.json");
    write_backup(&record_with(Some(document()), true), &backup, &files).expect("writes the backup");
    let restored = restore_claude_settings(&settings, &backup, &files)
        .expect("restores")
        .expect("a record");
    assert!(restored.existed);
    let written: SettingsDocument =
        serde_json::from_slice(&fs::read(&settings).expect("the settings")).expect("valid JSON");
    assert_eq!(written, document());
    assert!(!backup.exists());
}

#[test]
fn restore_removes_the_settings_when_it_did_not_exist() {
    let (directory, files) = restore_fixture();
    let settings = directory.path().join("settings.json");
    let backup = directory.path().join("backup.json");
    fs::write(&settings, b"{}").expect("the settings");
    write_backup(&record_with(None, false), &backup, &files).expect("writes the backup");
    let restored = restore_claude_settings(&settings, &backup, &files)
        .expect("restores")
        .expect("a record");
    assert!(!restored.existed);
    assert!(!settings.exists());
    assert!(!backup.exists());
}

#[test]
fn restore_null_content_with_existed_true_removes_the_settings() {
    let (directory, files) = restore_fixture();
    let settings = directory.path().join("settings.json");
    let backup = directory.path().join("backup.json");
    fs::write(&settings, b"{}").expect("the settings");
    fs::write(&backup, br#"{"existed":true,"content":null}"#).expect("the backup");
    let restored = restore_claude_settings(&settings, &backup, &files)
        .expect("restores")
        .expect("a record");
    assert!(restored.existed);
    assert!(restored.content.is_none());
    assert!(!settings.exists());
    assert!(!backup.exists());
}

#[test]
fn restore_without_a_backup_returns_none() {
    let (directory, files) = restore_fixture();
    let settings = directory.path().join("settings.json");
    fs::write(&settings, b"{}").expect("the settings");
    let backup = directory.path().join("backup.json");
    assert!(
        restore_claude_settings(&settings, &backup, &files)
            .expect("nothing to restore")
            .is_none()
    );
    assert!(settings.exists());
}

#[test]
fn restore_keeps_the_backup_when_the_write_fails() {
    let (directory, files) = restore_fixture();
    fs::write(directory.path().join(".claude"), b"not a directory").expect("blocking file");
    let settings = directory.path().join(".claude").join("settings.json");
    let backup = directory.path().join("backup.json");
    write_backup(&record_with(Some(document()), true), &backup, &files).expect("writes the backup");
    let error = restore_claude_settings(&settings, &backup, &files).expect_err("fails");
    assert!(matches!(error, BackupError::SettingsDirectory { .. }));
    assert!(backup.exists());
}

#[test]
fn restore_keeps_the_backup_when_the_settings_cannot_be_removed() {
    let (directory, files) = restore_fixture();
    let settings = directory.path().join("settings");
    fs::create_dir_all(&settings).expect("the settings directory");
    fs::write(settings.join("kept"), b"x").expect("a file inside");
    let backup = directory.path().join("backup.json");
    write_backup(&record_with(None, false), &backup, &files).expect("writes the backup");
    let error = restore_claude_settings(&settings, &backup, &files).expect_err("fails");
    assert!(matches!(error, BackupError::RemoveSettings { .. }));
    assert!(backup.exists());
}

#[test]
fn down_help_text_matches_litellm() {
    assert_eq!(
        DOWN_ABOUT,
        "Restore ~/.claude/settings.json if a `lite up` session left it patched"
    );
    assert_eq!(
        DOWN_LONG_ABOUT,
        "Restore ~/.claude/settings.json if a `lite up` session left it patched.\n\nUse this after a `lite up` process was killed uncleanly (e.g. `kill -9`) instead of stopped with Ctrl-C."
    );
}

#[test]
fn down_prints_nothing_to_restore() {
    let home = TempDir::new().expect("home");
    let environment = TestEnvironment::at(home.path());
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    down(&environment, &files, &mut stdout).expect("reports");
    assert_eq!(text(&stdout), "Nothing to restore.\n");
}

#[test]
fn down_prints_the_restored_line() {
    let home = TempDir::new().expect("home");
    let paths = claude_settings_paths(home.path());
    let files = PrivateFiles::default();
    write_backup(&record_with(Some(document()), true), &paths.backup, &files).expect("the backup");
    let environment = TestEnvironment::at(home.path());
    let mut stdout = Vec::new();
    down(&environment, &files, &mut stdout).expect("restores");
    assert_eq!(
        text(&stdout),
        format!(
            "Restored {} to its original contents.\n",
            paths.settings.display()
        )
    );
    assert!(!paths.backup.exists());
}

#[test]
fn down_prints_the_removed_line() {
    let home = TempDir::new().expect("home");
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.settings.parent().expect("a parent")).expect("settings directory");
    fs::write(&paths.settings, b"{}").expect("the settings");
    let files = PrivateFiles::default();
    write_backup(&record_with(None, false), &paths.backup, &files).expect("the backup");
    let environment = TestEnvironment::at(home.path());
    let mut stdout = Vec::new();
    down(&environment, &files, &mut stdout).expect("removes");
    assert_eq!(
        text(&stdout),
        format!(
            "Removed {} (it did not exist before `lite up`).\n",
            paths.settings.display()
        )
    );
    assert!(!paths.settings.exists());
}

#[test]
fn down_prints_the_restored_line_for_an_existed_null_backup() {
    let home = TempDir::new().expect("home");
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.settings.parent().expect("a parent")).expect("settings directory");
    fs::write(&paths.settings, b"{}").expect("the settings");
    fs::create_dir_all(paths.backup.parent().expect("a parent")).expect("backup directory");
    fs::write(&paths.backup, br#"{"existed":true,"content":null}"#).expect("the backup");
    let files = PrivateFiles::default();
    let environment = TestEnvironment::at(home.path());
    let mut stdout = Vec::new();
    down(&environment, &files, &mut stdout).expect("removes");
    assert_eq!(
        text(&stdout),
        format!(
            "Restored {} to its original contents.\n",
            paths.settings.display()
        )
    );
    assert!(!paths.settings.exists());
}

#[test]
fn down_ignores_the_config_dir() {
    let home = TempDir::new().expect("home");
    let config_dir = TempDir::new().expect("config directory");
    let config = config_dir.path().to_str().expect("a UTF-8 config path");
    let paths = claude_settings_paths(home.path());
    let files = PrivateFiles::default();
    write_backup(&record_with(Some(document()), true), &paths.backup, &files).expect("the backup");
    let environment =
        TestEnvironment::with_variables(home.path(), &[("CLAUDE_CONFIG_DIR", config)]);
    let mut stdout = Vec::new();
    down(&environment, &files, &mut stdout).expect("restores");
    assert!(paths.settings.exists());
    assert!(!config_dir.path().join("settings.json").exists());
}

#[test]
fn down_returns_the_restore_error() {
    let home = TempDir::new().expect("home");
    let paths = claude_settings_paths(home.path());
    fs::create_dir_all(paths.backup.parent().expect("a parent")).expect("backup directory");
    fs::write(&paths.backup, b"not json").expect("the backup");
    let files = PrivateFiles::default();
    let environment = TestEnvironment::at(home.path());
    let mut stdout = Vec::new();
    let error = down(&environment, &files, &mut stdout).expect_err("fails");
    assert!(matches!(error, BackupError::Invalid { .. }));
    assert!(stdout.is_empty());
}

#[test]
fn down_requires_a_home() {
    let files = PrivateFiles::default();
    let mut stdout = Vec::new();
    let error = down(&TestEnvironment::homeless(), &files, &mut stdout).expect_err("fails");
    assert!(matches!(error, BackupError::NoHome));
}

#[test]
fn login_probe_usable_login_needs_a_key() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    seed(&files, &record());
    let mut vault = FakeVault::unreachable_read();
    assert!(!usable_login(None, &files, &mut vault, 1_000.0_f64, 24).expect("checks the login"));
}

#[test]
fn login_probe_usable_login_needs_a_stored_credential() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut vault = FakeVault::unreachable_read();
    let key = secret("sk-context");
    assert!(
        !usable_login(Some(&key), &files, &mut vault, 1_000.0_f64, 24).expect("checks the login")
    );
}

#[rstest]
#[case::fresh(None, true)]
#[case::expired(Some(500.0_f64), false)]
#[case::inside_the_buffer(Some(1_200.0_f64), false)]
#[case::past_the_buffer(Some(1_500.0_f64), true)]
fn login_probe_usable_login_follows_freshness(
    #[case] expires_at: Option<f64>,
    #[case] usable: bool,
) {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut stored = record();
    stored.expires_at = expires_at;
    seed(&files, &stored);
    let mut vault = FakeVault::unreachable_read();
    let key = secret("sk-context");
    assert_eq!(
        usable_login(Some(&key), &files, &mut vault, 1_000.0_f64, 24).expect("checks the login"),
        usable
    );
}

fn context(from_token_file: bool) -> CliContext {
    CliContext {
        base_url: SERVER.to_owned(),
        base_url_explicit: from_token_file,
        api_key: Some(secret("sk-context")),
        api_key_from_token_file: from_token_file,
    }
}

struct ProbeFixture {
    files: ManagedTokenFile,
    vault: FakeVault,
    warnings: Mutex<Vec<String>>,
}

impl ProbeFixture {
    fn new(storage: ManagedTokenFile) -> Self {
        Self {
            files: storage,
            vault: FakeVault::unreachable_read(),
            warnings: Mutex::new(Vec::new()),
        }
    }

    fn resolved(
        &mut self,
        context: &CliContext,
        base_url: &str,
        http: Arc<FakeHttp>,
        now: i64,
    ) -> Option<SecretString> {
        let warn = |message: &str| {
            self.warnings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message.to_owned());
        };
        let request = KeyResolvedRequest {
            context,
            base_url,
            files: &self.files,
            vault: &mut self.vault,
            http,
            clock: Arc::new(FixedWallClock(now)),
            warn: &warn,
            token_path: Path::new("/home/tester/.litellm/token.json"),
        };
        key_resolved_on_the_way_in(request).expect("resolves the key")
    }

    fn warnings(&self) -> Vec<String> {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[test]
fn login_probe_key_resolved_uses_the_context_key_from_the_token_file() {
    let directory = TempDir::new().expect("directory");
    let mut fixture = ProbeFixture::new(token_file(directory.path()));
    let context = context(true);
    let key = fixture
        .resolved(&context, SERVER, Arc::new(FakeHttp::new(Vec::new())), 1_000)
        .expect("a key");
    assert_eq!(exposed(&key), "sk-context");
}

#[test]
fn login_probe_key_resolved_reads_the_stored_key_for_that_server() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    seed(&files, &record());
    let mut fixture = ProbeFixture::new(files);
    let context = context(false);
    let key = fixture
        .resolved(&context, SERVER, Arc::new(FakeHttp::new(Vec::new())), 1_000)
        .expect("a key");
    assert_eq!(exposed(&key), "sk-secret");
    assert!(fixture.warnings().is_empty());
}

#[test]
fn login_probe_key_resolved_rejects_a_key_from_another_server() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    seed(&files, &record());
    let mut fixture = ProbeFixture::new(files);
    let context = context(false);
    assert!(
        fixture
            .resolved(
                &context,
                OTHER_SERVER,
                Arc::new(FakeHttp::new(Vec::new())),
                1_000
            )
            .is_none()
    );
}

#[test]
fn login_probe_key_resolved_renews_a_stale_credential() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut stored = record();
    stored.expires_at = Some(900.0_f64);
    seed(&files, &stored);
    let mut fixture = ProbeFixture::new(files);
    let context = context(false);
    let http = Arc::new(FakeHttp::new(vec![Reply::Answer(token_response(
        "sk-renewed",
        "refresh-2",
        3_600,
    ))]));
    let key = fixture
        .resolved(&context, SERVER, Arc::clone(&http), 1_000)
        .expect("a key");
    assert_eq!(exposed(&key), "sk-renewed");
    assert_eq!(http.requests(), vec![TOKEN_ENDPOINT.to_owned()]);
}

#[rstest]
#[case::with_refresh_token(true)]
#[case::without_refresh_token(false)]
fn login_probe_stored_login_is_pkce_follows_the_refresh_token(#[case] renewable: bool) {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut stored = record();
    if !renewable {
        stored.refresh_token = None;
    }
    seed(&files, &stored);
    let mut vault = FakeVault::unreachable_read();
    assert_eq!(
        stored_login_is_pkce(&files, &mut vault).expect("checks the login"),
        renewable
    );
}

#[test]
fn login_probe_stored_login_is_pkce_needs_a_stored_credential() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut vault = FakeVault::unreachable_read();
    assert!(!stored_login_is_pkce(&files, &mut vault).expect("checks the login"));
}

#[test]
fn login_probe_stored_login_is_pkce_ignores_an_unreadable_vault() {
    let directory = TempDir::new().expect("directory");
    let files = token_file(directory.path());
    let mut stored = record();
    stored.refresh_token = None;
    seed(&files, &stored);
    let mut vault = FakeVault::unreachable_read();
    vault.read_failure = Some(KeyringUnusable::KeyringUnreachable);
    assert!(!stored_login_is_pkce(&files, &mut vault).expect("checks the login"));
}

#[test]
fn login_probe_key_resolved_returns_none_without_a_stored_credential() {
    let directory = TempDir::new().expect("directory");
    let mut fixture = ProbeFixture::new(token_file(directory.path()));
    let context = context(false);
    assert!(
        fixture
            .resolved(&context, SERVER, Arc::new(FakeHttp::new(Vec::new())), 1_000)
            .is_none()
    );
}
