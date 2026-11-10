#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::StagedFile;
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

use super::configure::{ConfigureRequest, configure_claude_settings};
use super::error::ReceiptsError;
use super::files::{FileFuture, SettingsFiles};
use super::preflight::preflight_claude_settings;
use super::read::read_configure_receipt;
use super::record::ConfigureReceipt;
use super::unconfigure::{UnconfigureOutcome, WithheldCredential, unconfigure_claude_settings};
use super::value::{ABSENT, OwnedValue, fingerprint};
use crate::claude_settings::choices::{ModelChoice, StaticToken};
use crate::claude_settings::error::ClaudeSettingsError;
use crate::claude_settings::paths::{
    ClaudeSettingsPaths, SettingsFileOwner, claude_settings_paths,
};
use crate::claude_settings::{
    ANTHROPIC_AUTH_TOKEN_KEY, ANTHROPIC_BASE_URL_KEY, ANTHROPIC_MODEL_KEY, ENV_KEY, MODEL_KEY,
    STATUS_LINE_KEY, SettingsDocument,
};

const BASE_URL: &str = "https://proxy.example.com";
const OLD_URL: &str = "https://user.example.com";
const TOKEN: &str = "sk-configured";
const BINARY: &str = "/opt/betterlite/lite";
const STATUS_COMMAND: &str = "/opt/betterlite/lite statusline";
const ORIGINAL: &str = r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user","ANTHROPIC_BASE_URL":"https://user.example.com","ENABLE_TOOL_SEARCH":"false","ANTHROPIC_API_KEY":"sk-stray"},"model":"user-model"}"#;
const USER_SETTINGS: &str =
    r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user","ANTHROPIC_BASE_URL":"https://user.example.com"}}"#;

struct Fixture {
    _home: TempDir,
    paths: ClaudeSettingsPaths,
    owners: Vec<SettingsFileOwner>,
}

impl Fixture {
    fn new() -> Self {
        let home = TempDir::new().unwrap();
        let paths = claude_settings_paths(home.path());
        Self {
            _home: home,
            paths,
            owners: Vec::new(),
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.paths.settings.clone()
    }

    fn state_path(&self) -> PathBuf {
        self.paths.configure_state.clone()
    }

    fn write_settings(&self, text: &str) {
        let document = serde_json::from_str::<SettingsDocument>(text).unwrap();
        self.write_document(&document);
    }

    fn write_document(&self, document: &SettingsDocument) {
        fs::create_dir_all(self.paths.settings.parent().unwrap()).unwrap();
        let bytes = serde_json::to_vec_pretty(document).unwrap();
        fs::write(&self.paths.settings, bytes).unwrap();
    }

    fn read_settings(&self) -> SettingsDocument {
        serde_json::from_slice(&fs::read(&self.paths.settings).unwrap()).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    EnsureDir(PathBuf),
    Stage(PathBuf),
    Commit(PathBuf),
    Discard(PathBuf),
    Remove(PathBuf),
}

#[derive(Default)]
struct Recording {
    operations: Vec<Operation>,
    staged: BTreeMap<PathBuf, JsonValue>,
    stage_paths: Vec<PathBuf>,
    commits: Vec<PathBuf>,
    fail_commits: Vec<usize>,
}

#[derive(Default)]
struct RecordingFiles {
    state: Mutex<Recording>,
}

impl RecordingFiles {
    fn new(fail_commits: &[usize]) -> Self {
        let files = Self::default();
        files.state.lock().unwrap().fail_commits = fail_commits.to_vec();
        files
    }

    fn operations(&self) -> Vec<Operation> {
        self.state.lock().unwrap().operations.clone()
    }
}

impl SettingsFiles for RecordingFiles {
    fn ensure_dir<'operation>(
        &'operation self,
        directory: &'operation Path,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state
                .operations
                .push(Operation::EnsureDir(directory.to_owned()));
            drop(state);
            Ok(())
        })
    }

    fn stage<'operation>(
        &'operation self,
        path: &'operation Path,
        document: &'operation JsonValue,
    ) -> FileFuture<'operation, StagedFile> {
        Box::pin(async move {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).unwrap();
            }
            let mut state = self.state.lock().unwrap();
            state.operations.push(Operation::Stage(path.to_owned()));
            state.stage_paths.push(path.to_owned());
            let staged = PathBuf::from(format!(".staged-{}", state.stage_paths.len()));
            state.staged.insert(staged.clone(), document.clone());
            drop(state);
            Ok(StagedFile { path: staged })
        })
    }

    fn commit<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
        path: &'operation Path,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.operations.push(Operation::Commit(path.to_owned()));
            state.commits.push(path.to_owned());
            let number = state.commits.len();
            if state.fail_commits.contains(&number) {
                drop(state);
                return Err(unwritable(path, "injected commit failure"));
            }
            let Some(document) = state.staged.remove(&staged.path) else {
                drop(state);
                return Err(unwritable(path, "the staged document is gone"));
            };
            drop(state);
            fs::write(path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
            Ok(())
        })
    }

    fn discard<'operation>(
        &'operation self,
        staged: &'operation StagedFile,
    ) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state
                .operations
                .push(Operation::Discard(staged.path.clone()));
            state.staged.remove(&staged.path);
            drop(state);
            Ok(())
        })
    }

    fn remove<'operation>(&'operation self, path: &'operation Path) -> FileFuture<'operation, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.operations.push(Operation::Remove(path.to_owned()));
            drop(state);
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(ReceiptsError::Remove {
                    path: path.to_owned(),
                    source: PrivateJsonError::Discard(path.to_owned(), source),
                }),
            }
        })
    }
}

fn unwritable(path: &Path, reason: &str) -> ReceiptsError {
    ReceiptsError::Write {
        path: path.to_owned(),
        source: PrivateJsonError::Commit(path.to_owned(), io::Error::other(reason.to_owned())),
    }
}

fn static_token(value: &str) -> StaticToken {
    StaticToken {
        token: SecretString::from(value),
    }
}

fn settings(text: &str) -> SettingsDocument {
    serde_json::from_str(text).unwrap()
}

fn owned(text: &str) -> OwnedValue {
    OwnedValue {
        present: true,
        value: JsonValue::String(text.to_owned()),
    }
}

fn env_text<'document>(document: &'document SettingsDocument, key: &str) -> Option<&'document str> {
    document.get(ENV_KEY)?.as_object()?.get(key)?.as_str()
}

fn set_env(document: &mut SettingsDocument, key: &str, value: &str) {
    let env = document
        .get_mut(ENV_KEY)
        .and_then(JsonValue::as_object_mut)
        .unwrap();
    env.insert(key.to_owned(), JsonValue::String(value.to_owned()));
}

fn status_command(document: &SettingsDocument) -> Option<&str> {
    document
        .get(STATUS_LINE_KEY)?
        .as_object()?
        .get("command")?
        .as_str()
}

fn stored_receipt(fixture: &Fixture) -> ConfigureReceipt {
    read_configure_receipt(&fixture.state_path())
        .unwrap()
        .unwrap()
}

async fn run_configure(
    fixture: &Fixture,
    files: &RecordingFiles,
    base_url: &str,
    credential: &StaticToken,
    model: &ModelChoice,
) -> Result<(), ReceiptsError> {
    let request = ConfigureRequest {
        base_url,
        credential,
        model,
        settings_path: &fixture.paths.settings,
        state_path: &fixture.paths.configure_state,
        owners: &fixture.owners,
        binary_path: Some(Path::new(BINARY)),
    };
    configure_claude_settings(&request, files).await
}

async fn configure(fixture: &Fixture, files: &RecordingFiles) -> Result<(), ReceiptsError> {
    let credential = static_token(TOKEN);
    run_configure(
        fixture,
        files,
        BASE_URL,
        &credential,
        &ModelChoice::KeepModel,
    )
    .await
}

async fn unconfigure(
    fixture: &Fixture,
    files: &RecordingFiles,
) -> Result<UnconfigureOutcome, ReceiptsError> {
    unconfigure_claude_settings(
        &fixture.paths.settings,
        &fixture.paths.configure_state,
        &fixture.owners,
        files,
    )
    .await
}

async fn configured(content: Option<&str>) -> (Fixture, RecordingFiles) {
    let fixture = Fixture::new();
    if let Some(text) = content {
        fixture.write_settings(text);
    }
    let files = RecordingFiles::new(&[]);
    configure(&fixture, &files).await.unwrap();
    (fixture, files)
}

#[rstest]
#[case::escaped_string(
    OwnedValue { present: true, value: JsonValue::String("\u{e9}\n\"".to_owned()) },
    "{\"present\": true, \"value\": \"\\u00e9\\n\\\"\"}"
)]
#[case::absent_value(ABSENT, r#"{"present": false, "value": null}"#)]
#[case::astral_string(
    OwnedValue { present: true, value: JsonValue::String("\u{1f600}".to_owned()) },
    "{\"present\": true, \"value\": \"\\ud83d\\ude00\"}"
)]
fn configure_fingerprint_matches_the_canonical_json(
    #[case] value: OwnedValue,
    #[case] canonical: &str,
) {
    assert_eq!(
        fingerprint(&value),
        hex::encode(Sha256::digest(canonical.as_bytes()))
    );
}

#[test]
fn configure_fingerprint_ignores_object_key_order() {
    let first = OwnedValue {
        present: true,
        value: serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap(),
    };
    let second = OwnedValue {
        present: true,
        value: serde_json::from_str(r#"{"a":2,"b":1}"#).unwrap(),
    };
    assert_eq!(fingerprint(&first), fingerprint(&second));
}

#[test]
fn configure_fingerprint_separates_an_absent_key_from_a_null_value() {
    let null = OwnedValue {
        present: true,
        value: JsonValue::Null,
    };
    assert_ne!(fingerprint(&ABSENT), fingerprint(&null));
}

#[tokio::test]
async fn configure_records_previous_values() {
    let (fixture, _files) = configured(Some(ORIGINAL)).await;
    let receipt = stored_receipt(&fixture);
    assert!(receipt.file_existed);
    assert!(receipt.env_present);
    assert!(receipt.env_was_object);
    assert_eq!(
        receipt.previous.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned("sk-user"))
    );
    assert_eq!(
        receipt.previous.get("env.ENABLE_TOOL_SEARCH"),
        Some(&owned("false"))
    );
    assert_eq!(receipt.previous.get(MODEL_KEY), Some(&owned("user-model")));
    assert_eq!(receipt.previous.get("env.ANTHROPIC_MODEL"), Some(&ABSENT));
}

#[tokio::test]
async fn configure_written_fingerprints_and_endpoints() {
    let (fixture, _files) = configured(Some(ORIGINAL)).await;
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_BASE_URL"),
        Some(&fingerprint(&owned(BASE_URL)))
    );
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&fingerprint(&owned(TOKEN)))
    );
    assert_eq!(
        receipt.written.get("env.ENABLE_TOOL_SEARCH"),
        Some(&fingerprint(&owned("true")))
    );
    assert_eq!(receipt.written.get(MODEL_KEY), None);
    assert_eq!(
        receipt.endpoints.get("env.ANTHROPIC_API_KEY"),
        Some(&owned(OLD_URL))
    );
    assert_eq!(
        receipt.endpoints.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned(OLD_URL))
    );
    assert_eq!(receipt.endpoints.get("apiKeyHelper"), Some(&owned(OLD_URL)));
    let document = fixture.read_settings();
    assert_eq!(env_text(&document, ANTHROPIC_BASE_URL_KEY), Some(BASE_URL));
    assert_eq!(env_text(&document, ANTHROPIC_AUTH_TOKEN_KEY), Some(TOKEN));
}

#[tokio::test]
async fn configure_keeps_the_first_previous_and_endpoint_across_repeats() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let credential = static_token("sk-second");
    run_configure(
        &fixture,
        &files,
        "https://second.example.com",
        &credential,
        &ModelChoice::KeepModel,
    )
    .await
    .unwrap();
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        receipt.previous.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned("sk-user"))
    );
    assert_eq!(
        receipt.previous.get("env.ANTHROPIC_BASE_URL"),
        Some(&owned(OLD_URL))
    );
    assert_eq!(
        receipt.endpoints.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned(OLD_URL))
    );
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&fingerprint(&owned("sk-second")))
    );
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_BASE_URL"),
        Some(&fingerprint(&owned("https://second.example.com")))
    );
}

#[tokio::test]
async fn configure_recaptures_previous_after_the_user_edits_a_setting() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let mut document = fixture.read_settings();
    set_env(&mut document, ANTHROPIC_AUTH_TOKEN_KEY, "sk-edited");
    fixture.write_document(&document);
    configure(&fixture, &files).await.unwrap();
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        receipt.previous.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned("sk-edited"))
    );
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&fingerprint(&owned(TOKEN)))
    );
}

#[tokio::test]
async fn configure_unpins_a_model_it_owns() {
    let fixture = Fixture::new();
    let files = RecordingFiles::new(&[]);
    let credential = static_token(TOKEN);
    let pinned = ModelChoice::StartOn("proxy-model".to_owned());
    run_configure(&fixture, &files, BASE_URL, &credential, &pinned)
        .await
        .unwrap();
    run_configure(
        &fixture,
        &files,
        BASE_URL,
        &credential,
        &ModelChoice::UnpinModel,
    )
    .await
    .unwrap();
    let document = fixture.read_settings();
    assert_eq!(document.get(MODEL_KEY), None);
    assert_eq!(env_text(&document, ANTHROPIC_MODEL_KEY), None);
    let receipt = stored_receipt(&fixture);
    assert_eq!(receipt.previous.get(MODEL_KEY), Some(&ABSENT));
    assert_eq!(receipt.written.get(MODEL_KEY), Some(&fingerprint(&ABSENT)));
}

#[tokio::test]
async fn configure_leaves_a_model_the_user_pinned_by_hand() {
    let fixture = Fixture::new();
    let files = RecordingFiles::new(&[]);
    let credential = static_token(TOKEN);
    let pinned = ModelChoice::StartOn("proxy-model".to_owned());
    run_configure(&fixture, &files, BASE_URL, &credential, &pinned)
        .await
        .unwrap();
    let mut document = fixture.read_settings();
    document.insert(
        MODEL_KEY.to_owned(),
        JsonValue::String("user-model".to_owned()),
    );
    fixture.write_document(&document);
    run_configure(
        &fixture,
        &files,
        BASE_URL,
        &credential,
        &ModelChoice::UnpinModel,
    )
    .await
    .unwrap();
    let after_reconfigure = fixture.read_settings();
    assert_eq!(
        after_reconfigure.get(MODEL_KEY),
        Some(&JsonValue::String("user-model".to_owned()))
    );
    assert_eq!(env_text(&after_reconfigure, ANTHROPIC_MODEL_KEY), None);
    assert_eq!(
        stored_receipt(&fixture).previous.get(MODEL_KEY),
        Some(&owned("user-model"))
    );
}

#[tokio::test]
async fn configure_stages_both_files_before_committing() {
    let (fixture, files) = configured(None).await;
    let state_dir = fixture
        .paths
        .configure_state
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(
        files.operations(),
        vec![
            Operation::EnsureDir(state_dir),
            Operation::Stage(fixture.state_path()),
            Operation::Stage(fixture.settings_path()),
            Operation::Commit(fixture.state_path()),
            Operation::Commit(fixture.settings_path()),
        ]
    );
}

#[tokio::test]
async fn configure_restores_the_earlier_receipt_when_the_settings_commit_fails() {
    let fixture = Fixture::new();
    fixture.write_settings(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user"}}"#);
    let first = RecordingFiles::new(&[]);
    configure(&fixture, &first).await.unwrap();
    let earlier_receipt = fs::read(fixture.state_path()).unwrap();
    let earlier_settings = fs::read(fixture.settings_path()).unwrap();
    let second = RecordingFiles::new(&[2]);
    let credential = static_token("sk-second");
    let error = run_configure(
        &fixture,
        &second,
        "https://second.example.com",
        &credential,
        &ModelChoice::KeepModel,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ReceiptsError::Write { .. }));
    assert_eq!(fs::read(fixture.state_path()).unwrap(), earlier_receipt);
    assert_eq!(fs::read(fixture.settings_path()).unwrap(), earlier_settings);
}

#[tokio::test]
async fn configure_removes_the_new_receipt_when_the_settings_commit_fails_without_an_earlier() {
    let fixture = Fixture::new();
    let files = RecordingFiles::new(&[2]);
    let error = configure(&fixture, &files).await.unwrap_err();
    assert!(matches!(error, ReceiptsError::Write { .. }));
    assert!(!fixture.state_path().exists());
    assert!(!fixture.settings_path().exists());
    assert_eq!(
        files.operations().last(),
        Some(&Operation::Remove(fixture.state_path()))
    );
}

#[tokio::test]
async fn configure_reports_an_inconsistent_state_when_the_receipt_cannot_be_restored() {
    let fixture = Fixture::new();
    fixture.write_settings(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user"}}"#);
    let first = RecordingFiles::new(&[]);
    configure(&fixture, &first).await.unwrap();
    let files = RecordingFiles::new(&[2, 3]);
    let credential = static_token("sk-second");
    let error = run_configure(
        &fixture,
        &files,
        "https://second.example.com",
        &credential,
        &ModelChoice::KeepModel,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        ReceiptsError::Inconsistent { state_path, .. } if *state_path == fixture.state_path()
    ));
    assert!(error.to_string().contains("could not be put back"));
    assert!(error.source().is_some());
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_BASE_URL"),
        Some(&fingerprint(&owned("https://second.example.com")))
    );
}

#[tokio::test]
async fn configure_refuses_while_lite_up_owns_the_settings() {
    let mut fixture = Fixture::new();
    fixture.write_settings("{}");
    let backup = fixture.paths.backup.clone();
    fs::create_dir_all(backup.parent().unwrap()).unwrap();
    fs::write(&backup, "{}").unwrap();
    fixture.owners = vec![SettingsFileOwner {
        backup_path: backup,
        start_command: "lite up",
        stop_command: "lite down",
    }];
    let files = RecordingFiles::new(&[]);
    let error = configure(&fixture, &files).await.unwrap_err();
    assert!(matches!(
        error,
        ReceiptsError::Settings(ClaudeSettingsError::Owned { .. })
    ));
    assert!(files.operations().is_empty());
}

#[tokio::test]
async fn configure_rejects_a_non_object_env_before_writing() {
    let fixture = Fixture::new();
    fixture.write_settings(r#"{"env":[]}"#);
    let files = RecordingFiles::new(&[]);
    let error = configure(&fixture, &files).await.unwrap_err();
    assert!(matches!(
        error,
        ReceiptsError::Settings(ClaudeSettingsError::NonObjectEnv { .. })
    ));
    assert!(files.operations().is_empty());
}

#[rstest]
#[case::absent_file(None, (false, false, false))]
#[case::empty_object(Some("{}"), (true, false, false))]
#[case::null_env(Some(r#"{"env":null}"#), (true, true, false))]
#[case::object_env(Some(r#"{"env":{"FOO":"bar"}}"#), (true, true, true))]
#[tokio::test]
async fn configure_records_the_file_and_env_flags(
    #[case] content: Option<&str>,
    #[case] expected: (bool, bool, bool),
) {
    let (fixture, _files) = configured(content).await;
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        (
            receipt.file_existed,
            receipt.env_present,
            receipt.env_was_object
        ),
        expected
    );
}

#[tokio::test]
async fn configure_leaves_a_user_status_line_unowned() {
    let (fixture, _files) = configured(Some(
        r#"{"statusLine":{"type":"command","command":"bun x ccusage statusline"}}"#,
    ))
    .await;
    assert_eq!(
        status_command(&fixture.read_settings()),
        Some("bun x ccusage statusline")
    );
    assert_eq!(stored_receipt(&fixture).written.get(STATUS_LINE_KEY), None);
}

#[tokio::test]
async fn configure_writes_the_status_line_when_the_user_has_none() {
    let (fixture, _files) = configured(None).await;
    assert_eq!(
        status_command(&fixture.read_settings()),
        Some(STATUS_COMMAND)
    );
    let value: JsonValue =
        serde_json::from_str(r#"{"command":"/opt/betterlite/lite statusline","type":"command"}"#)
            .unwrap();
    assert_eq!(
        stored_receipt(&fixture).written.get(STATUS_LINE_KEY),
        Some(&fingerprint(&OwnedValue {
            present: true,
            value,
        }))
    );
}

#[tokio::test]
async fn unconfigure_restores_owned_paths_to_their_previous_values() {
    let (fixture, files) = configured(Some(ORIGINAL)).await;
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    assert_eq!(fixture.read_settings(), settings(ORIGINAL));
    assert!(outcome.restored.contains(&"statusLine".to_owned()));
    assert!(
        outcome
            .restored
            .contains(&"env.ANTHROPIC_AUTH_TOKEN".to_owned())
    );
    assert!(
        outcome
            .restored
            .contains(&"env.ANTHROPIC_BASE_URL".to_owned())
    );
    assert!(
        outcome
            .restored
            .contains(&"env.CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY".to_owned())
    );
    assert!(outcome.kept.is_empty());
    assert!(outcome.withheld.is_empty());
    assert!(!outcome.file_removed);
    assert!(!fixture.state_path().exists());
}

#[tokio::test]
async fn unconfigure_restores_only_fingerprint_matches_and_reports_kept_edits() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let mut edited = fixture.read_settings();
    set_env(&mut edited, ANTHROPIC_AUTH_TOKEN_KEY, "sk-edited");
    fixture.write_document(&edited);
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    let restored = fixture.read_settings();
    assert_eq!(
        env_text(&restored, ANTHROPIC_AUTH_TOKEN_KEY),
        Some("sk-edited")
    );
    assert_eq!(env_text(&restored, ANTHROPIC_BASE_URL_KEY), Some(OLD_URL));
    assert!(
        outcome
            .kept
            .contains(&"env.ANTHROPIC_AUTH_TOKEN".to_owned())
    );
    assert!(
        !outcome
            .restored
            .contains(&"env.ANTHROPIC_AUTH_TOKEN".to_owned())
    );
}

#[tokio::test]
async fn unconfigure_restores_nothing_and_removes_the_settings_file_it_created() {
    let (fixture, files) = configured(None).await;
    assert!(fixture.settings_path().exists());
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    assert!(outcome.file_removed);
    assert!(!fixture.settings_path().exists());
    assert!(!fixture.state_path().exists());
}

#[rstest]
#[case::empty_object("{}", "{}")]
#[case::empty_env(r#"{"env":{}}"#, r#"{"env":{}}"#)]
#[case::null_env(r#"{"env":null}"#, r#"{"env":null}"#)]
#[case::other_keys(
    r#"{"permissions":{"allow":["Bash"]}}"#,
    r#"{"permissions":{"allow":["Bash"]}}"#
)]
#[tokio::test]
async fn unconfigure_restores_the_minimal_remaining_document(
    #[case] content: &str,
    #[case] expected: &str,
) {
    let (fixture, files) = configured(Some(content)).await;
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    assert_eq!(fixture.read_settings(), settings(expected));
    assert!(!outcome.file_removed);
}

#[tokio::test]
async fn unconfigure_restores_a_credential_with_no_endpoint_on_either_side() {
    let (fixture, files) = configured(Some(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user"}}"#)).await;
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    assert!(outcome.withheld.is_empty());
    assert_eq!(
        fixture.read_settings(),
        settings(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user"}}"#)
    );
}

#[tokio::test]
async fn unconfigure_withholds_a_credential_when_the_url_points_elsewhere() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let mut edited = fixture.read_settings();
    set_env(
        &mut edited,
        ANTHROPIC_BASE_URL_KEY,
        "https://other.example.com",
    );
    fixture.write_document(&edited);
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    let restored = fixture.read_settings();
    assert_eq!(env_text(&restored, ANTHROPIC_AUTH_TOKEN_KEY), None);
    assert_eq!(
        env_text(&restored, ANTHROPIC_BASE_URL_KEY),
        Some("https://other.example.com")
    );
    assert_eq!(
        outcome.withheld,
        vec![WithheldCredential {
            key: "env.ANTHROPIC_AUTH_TOKEN".to_owned(),
            endpoint: OLD_URL.to_owned(),
        }]
    );
    assert!(outcome.kept.contains(&"env.ANTHROPIC_BASE_URL".to_owned()));
    assert!(
        outcome
            .restored
            .contains(&"env.ANTHROPIC_AUTH_TOKEN".to_owned())
    );
}

#[tokio::test]
async fn withheld_credentials_leave_a_trimmed_receipt() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let mut edited = fixture.read_settings();
    set_env(
        &mut edited,
        ANTHROPIC_BASE_URL_KEY,
        "https://other.example.com",
    );
    fixture.write_document(&edited);
    unconfigure(&fixture, &files).await.unwrap();
    let receipt = stored_receipt(&fixture);
    assert_eq!(
        receipt.written.keys().cloned().collect::<Vec<String>>(),
        vec!["env.ANTHROPIC_AUTH_TOKEN".to_owned()]
    );
    assert_eq!(
        receipt.written.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&fingerprint(&ABSENT))
    );
    assert_eq!(
        receipt.endpoints.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned(OLD_URL))
    );
    assert_eq!(
        receipt.previous.get("env.ANTHROPIC_AUTH_TOKEN"),
        Some(&owned("sk-user"))
    );
}

#[tokio::test]
async fn withheld_credentials_report_the_endpoint_they_were_issued_for() {
    let (fixture, files) = configured(Some(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-user"}}"#)).await;
    let mut edited = fixture.read_settings();
    set_env(
        &mut edited,
        ANTHROPIC_BASE_URL_KEY,
        "https://other.example.com",
    );
    fixture.write_document(&edited);
    let outcome = unconfigure(&fixture, &files).await.unwrap();
    assert_eq!(
        outcome.withheld,
        vec![WithheldCredential {
            key: "env.ANTHROPIC_AUTH_TOKEN".to_owned(),
            endpoint: "no ANTHROPIC_BASE_URL (Anthropic's default endpoint)".to_owned(),
        }]
    );
}

#[tokio::test]
async fn withheld_credentials_stay_in_a_receipt_a_later_unconfigure_restores() {
    let (fixture, files) = configured(Some(USER_SETTINGS)).await;
    let mut edited = fixture.read_settings();
    set_env(
        &mut edited,
        ANTHROPIC_BASE_URL_KEY,
        "https://other.example.com",
    );
    fixture.write_document(&edited);
    let first = unconfigure(&fixture, &files).await.unwrap();
    assert_eq!(first.withheld.len(), 1);
    let mut repointed = fixture.read_settings();
    set_env(&mut repointed, ANTHROPIC_BASE_URL_KEY, OLD_URL);
    fixture.write_document(&repointed);
    let second = unconfigure(&fixture, &files).await.unwrap();
    assert_eq!(
        env_text(&fixture.read_settings(), ANTHROPIC_AUTH_TOKEN_KEY),
        Some("sk-user")
    );
    assert!(second.withheld.is_empty());
    assert!(
        second
            .restored
            .contains(&"env.ANTHROPIC_AUTH_TOKEN".to_owned())
    );
    assert!(!fixture.state_path().exists());
}

#[test]
fn read_configure_receipt_returns_none_when_there_is_no_receipt() {
    let fixture = Fixture::new();
    let receipt = read_configure_receipt(&fixture.state_path()).unwrap();
    assert!(receipt.is_none());
}

#[rstest]
#[case::garbage("not json")]
#[case::array("[]")]
#[case::null("null")]
#[case::empty_object("{}")]
#[case::missing_maps(r#"{"file_existed":true,"env_present":false,"env_was_object":false}"#)]
fn read_configure_receipt_rejects_a_file_that_is_not_a_receipt(#[case] content: &str) {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.state_path().parent().unwrap()).unwrap();
    fs::write(fixture.state_path(), content).unwrap();
    let error = read_configure_receipt(&fixture.state_path()).unwrap_err();
    assert!(matches!(error, ReceiptsError::InvalidReceipt { .. }));
    assert!(error.source().is_some());
}

#[test]
fn read_configure_receipt_reports_an_unreadable_file() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.state_path()).unwrap();
    let error = read_configure_receipt(&fixture.state_path()).unwrap_err();
    assert!(matches!(error, ReceiptsError::ReadReceipt { .. }));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn read_configure_receipt_makes_unconfigure_report_nothing_to_undo() {
    let fixture = Fixture::new();
    let files = RecordingFiles::new(&[]);
    let error = unconfigure(&fixture, &files).await.unwrap_err();
    assert!(matches!(error, ReceiptsError::NoReceipt { .. }));
    assert!(error.to_string().contains("nothing to undo"));
    assert!(files.operations().is_empty());
}

#[test]
fn preflight_claude_settings_allows_the_first_configure() {
    let fixture = Fixture::new();
    preflight_claude_settings(&fixture.settings_path(), &fixture.paths).unwrap();
}

#[test]
fn preflight_claude_settings_surfaces_an_invalid_receipt() {
    let fixture = Fixture::new();
    fixture.write_settings("{}");
    fs::create_dir_all(fixture.state_path().parent().unwrap()).unwrap();
    fs::write(fixture.state_path(), "stale receipt").unwrap();
    let error = preflight_claude_settings(&fixture.settings_path(), &fixture.paths).unwrap_err();
    assert!(matches!(error, ReceiptsError::InvalidReceipt { .. }));
}

#[test]
fn preflight_claude_settings_rejects_a_non_object_env() {
    let fixture = Fixture::new();
    fixture.write_settings(r#"{"env":"text"}"#);
    let error = preflight_claude_settings(&fixture.settings_path(), &fixture.paths).unwrap_err();
    assert!(matches!(
        error,
        ReceiptsError::Settings(ClaudeSettingsError::NonObjectEnv { .. })
    ));
}

#[test]
fn preflight_claude_settings_refuses_while_an_owner_holds_a_backup() {
    let mut fixture = Fixture::new();
    fixture.write_settings("{}");
    let backup = fixture.paths.backup.clone();
    fs::create_dir_all(backup.parent().unwrap()).unwrap();
    fs::write(&backup, "{}").unwrap();
    fixture.owners = vec![SettingsFileOwner {
        backup_path: backup,
        start_command: "lite up",
        stop_command: "lite down",
    }];
    let error = preflight_claude_settings(&fixture.settings_path(), &fixture.paths).unwrap_err();
    assert!(matches!(
        error,
        ReceiptsError::Settings(ClaudeSettingsError::Owned { .. })
    ));
}
