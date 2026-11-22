#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use betterlite_models::decisions::Json;
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use chrono::{DateTime, FixedOffset};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::NamedTempFile;
use tokio::task::yield_now;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit_events::callbacks::{
    AuditCallbackDispatcher, AuditCallbackError, AuditCallbackFuture, AuditCallbackRegistry,
    AuditLogCallback, AuditTaskSpawner,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::ConfiguredAuditSettings;
use crate::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError,
};
use crate::member_request_contracts::callbacks::{CallbackSecrets, TeamCallbackMetadata};
use crate::team_callback_policy::audit::{TeamAuditWriter, TeamCallbackAudit, TeamCallbackChange};
use crate::team_callback_policy::error::{
    ErrorDetail, HttpFailure, TeamCallbackError, unknown_team_error,
};
use crate::team_callback_policy::redaction::redact_callback_secrets;
use crate::team_callback_policy::resolve::resolve_team_callbacks;
use crate::team_callback_policy::{CALLBACK_VARS_REDACTED, validate_team_callback};

const SALT_KEY: &str = "team-callback-policy-salt";
const AUDIT_ID: &str = "audit-row-1";
const NOW: &str = "2026-11-21T10:30:00+00:00";
const FLAGS: FeatureFlags = FeatureFlags {
    otel_v2_enabled: false,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn vars(pairs: &[(&str, &str)]) -> CallbackVars {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolved(lists: [&[&str]; 3], pairs: &[(&str, &str)]) -> TeamCallbackMetadata {
    let [success, failure, callbacks] = lists.map(|names| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    });
    let secrets: BTreeMap<String, String> = vars(pairs).into_iter().collect();
    TeamCallbackMetadata {
        success_callback: success,
        failure_callback: failure,
        callbacks,
        callback_vars: CallbackSecrets::from(secrets),
    }
}

fn cipher() -> DbCipher {
    let signing_key = Some(SecretString::from(SALT_KEY));
    let settings = CipherSettings {
        signing_key,
        algorithm: Algorithm::XSalsa20Poly1305,
    };
    StoredCipher {
        settings,
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn sealed(metadata: &Json) -> Json {
    let plain = serde_json::to_value(metadata).unwrap();
    serde_json::from_value(transform_callback_vars(
        &plain,
        &Transform::Encrypt(&cipher()),
    ))
    .unwrap()
}

fn resolve(metadata: &Json) -> Result<TeamCallbackMetadata, TeamCallbackError> {
    resolve_team_callbacks(metadata, &cipher())
}

fn capture_warnings() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let writer = Arc::new(log.reopen().unwrap());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(writer)
        .finish();
    (log, set_default(subscriber))
}

fn warned(log: &NamedTempFile, message: &str) -> usize {
    fs::read_to_string(log.path())
        .unwrap()
        .matches(message)
        .count()
}

#[rstest]
#[case::langfuse_environment("langfuse", &[("langfuse_environment", "Prod Env")])]
#[case::arize_rate_range("arize", &[("arize_success_sampling_rate", "1.5")])]
#[case::span_scope_callback("langfuse", &[("langfuse_span_scope", "full")])]
fn invalid_callback_400(#[case] name: &str, #[case] pairs: &[(&str, &str)]) {
    let error = validate_team_callback(name, &vars(pairs), FLAGS).unwrap_err();
    let failure = error.failure().unwrap();
    assert_eq!(failure.status, StatusCode::BAD_REQUEST);
    let ErrorDetail::Error { error: message } = failure.detail else {
        panic!("a rejected callback answers with detail.error");
    };
    assert!(!message.is_empty());
}

#[rstest]
#[case::no_vars("langfuse", &[])]
#[case::langfuse_environment("langfuse", &[("langfuse_environment", "prod-eu")])]
#[case::arize_rate("arize", &[("arize_success_sampling_rate", "0.5")])]
fn valid_callback(#[case] name: &str, #[case] pairs: &[(&str, &str)]) {
    validate_team_callback(name, &vars(pairs), FLAGS).unwrap();
}

#[test]
fn audit_redacts_all_vars() {
    let metadata = json(
        r#"{"owner": "ops", "logging": [{"callback_name": "langfuse", "callback_type": "success", "callback_vars": {"langfuse_host": "https://lf.example", "langfuse_secret_key": "sk-1"}}], "callback_settings": {"success_callback": ["langsmith"], "callback_vars": {"langsmith_project": "proj", "langsmith_api_key": "ls-1"}}}"#,
    );
    let expected = json(
        r#"{"owner": "ops", "logging": [{"callback_name": "langfuse", "callback_type": "success", "callback_vars": {"langfuse_host": "***REDACTED***", "langfuse_secret_key": "***REDACTED***"}}], "callback_settings": {"success_callback": ["langsmith"], "callback_vars": {"langsmith_project": "***REDACTED***", "langsmith_api_key": "***REDACTED***"}}}"#,
    );
    assert_eq!(redact_callback_secrets(&metadata), expected);
}

#[test]
fn redaction_deep_copy() {
    let stored = json(
        r#"{"logging": [{"callback_name": "langsmith", "callback_vars": {"langsmith_api_key": "ls-1", "langsmith_project": "proj"}}], "callback_settings": {"callback_vars": {"langfuse_secret_key": "sk-1"}}}"#,
    );
    let untouched = stored.clone();
    let redacted = redact_callback_secrets(&stored);
    let masked = resolve(&stored).unwrap();
    assert_ne!(redacted, stored);
    assert_eq!(stored, untouched);
    assert_eq!(
        masked,
        resolved(
            [&["langsmith"], &["langsmith"], &["langsmith"]],
            &[
                ("langsmith_api_key", CALLBACK_VARS_REDACTED),
                ("langsmith_project", "proj")
            ],
        )
    );
}

#[rstest]
#[case::not_an_object(r#"["logging"]"#)]
#[case::text(r#""metadata""#)]
#[case::null("null")]
#[case::logging_not_a_list(r#"{"logging": {"callback_vars": {"langfuse_secret_key": "sk-1"}}}"#)]
#[case::entry_not_an_object(r#"{"logging": ["langfuse", 3, null]}"#)]
#[case::entry_vars_not_an_object(r#"{"logging": [{"callback_vars": ["sk-1"]}]}"#)]
#[case::settings_not_an_object(r#"{"callback_settings": "sk-1"}"#)]
#[case::settings_vars_not_an_object(r#"{"callback_settings": {"callback_vars": "sk-1"}}"#)]
fn malformed_slots_unchanged(#[case] text: &str) {
    let metadata = json(text);
    assert_eq!(redact_callback_secrets(&metadata), metadata);
}

#[test]
fn sensitive_keys_masked() {
    let stored = sealed(&json(
        r#"{"logging": [{"callback_name": "langfuse", "callback_vars": {"langfuse_public_key": "pk-1", "langfuse_secret_key": "sk-1", "gcs_path_service_account": "/keys/sa.json"}}]}"#,
    ));
    assert!(
        serde_json::to_string(&stored)
            .unwrap()
            .contains("litellm_enc::")
    );
    let masked = resolve(&stored).unwrap();
    assert_eq!(
        masked,
        resolved(
            [&["langfuse"], &["langfuse"], &["langfuse"]],
            &[
                ("gcs_path_service_account", CALLBACK_VARS_REDACTED),
                ("langfuse_public_key", CALLBACK_VARS_REDACTED),
                ("langfuse_secret_key", CALLBACK_VARS_REDACTED)
            ],
        )
    );
}

#[rstest]
#[case::ordinary_key("langsmith_project")]
#[case::sensitive_key("langsmith_api_key")]
fn undecrypted_prefix_masked(#[case] name: &str) {
    let stored = json(&format!(
        r#"{{"logging": [{{"callback_name": "langsmith", "callback_vars": {{"{name}": "litellm_enc::not-a-ciphertext", "langsmith_base_url": "https://ls.example"}}}}]}}"#
    ));
    let masked = resolve(&stored).unwrap();
    assert_eq!(
        masked,
        resolved(
            [&["langsmith"], &["langsmith"], &["langsmith"]],
            &[
                (name, CALLBACK_VARS_REDACTED),
                ("langsmith_base_url", "https://ls.example")
            ],
        )
    );
}

#[test]
fn ordinary_vars_visible() {
    let stored = sealed(&json(
        r#"{"logging": [{"callback_name": "gcs_bucket", "callback_type": "success", "callback_vars": {"gcs_bucket_name": "audit-bucket", "langfuse_host": "https://lf.example", "langsmith_project": "proj"}}]}"#,
    ));
    let masked = resolve(&stored).unwrap();
    assert_eq!(
        masked,
        resolved(
            [&["gcs_bucket"], &[], &[]],
            &[
                ("gcs_bucket_name", "audit-bucket"),
                ("langfuse_host", "https://lf.example"),
                ("langsmith_project", "proj")
            ],
        )
    );
}

#[rstest]
#[case::null("null")]
#[case::list(r#"[{"logging": []}]"#)]
#[case::text(r#""callback_settings""#)]
#[case::number("7")]
#[case::empty_object("{}")]
fn nonobject_defaults(#[case] text: &str) {
    assert_eq!(
        resolve(&json(text)).unwrap(),
        TeamCallbackMetadata::default()
    );
}

const LEGACY: &str = r#""callback_settings": {"success_callback": ["langfuse"], "failure_callback": null, "callback_vars": {"langfuse_secret_key": "sk-legacy", "langfuse_host": "https://lf.example"}}"#;

#[test]
fn logging_empty_shadows_legacy() {
    let metadata = json(&format!(r#"{{"logging": [], {LEGACY}}}"#));
    assert_eq!(resolve(&metadata).unwrap(), TeamCallbackMetadata::default());
}

#[rstest]
#[case::text(r#""langfuse""#)]
#[case::object(r#"{"callback_name": "langfuse", "callback_vars": {}}"#)]
#[case::number("1")]
fn logging_nonlist_shadows_legacy(#[case] logging: &str) {
    let metadata = json(&format!(r#"{{"logging": {logging}, {LEGACY}}}"#));
    assert_eq!(resolve(&metadata).unwrap(), TeamCallbackMetadata::default());
}

#[rstest]
#[case::absent("")]
#[case::null(r#""logging": null, "#)]
fn null_logging_uses_legacy(#[case] logging: &str) {
    let masked = resolve(&sealed(&json(&format!("{{{logging}{LEGACY}}}")))).unwrap();
    assert_eq!(
        masked,
        resolved(
            [&["langfuse"], &[], &[]],
            &[
                ("langfuse_host", "https://lf.example"),
                ("langfuse_secret_key", CALLBACK_VARS_REDACTED)
            ],
        )
    );
}

#[rstest]
#[case::text(r#"{"callback_settings": "langfuse"}"#)]
#[case::list(r#"{"logging": null, "callback_settings": ["langfuse"]}"#)]
#[case::all_null(r#"{"callback_settings": {"success_callback": null, "callback_vars": null}}"#)]
fn legacy_nonobject_defaults(#[case] text: &str) {
    assert_eq!(
        resolve(&json(text)).unwrap(),
        TeamCallbackMetadata::default()
    );
}

#[test]
fn invalid_entry_warn_drop() {
    let metadata = json(
        r#"{"logging": [{"callback_name": "langsmith", "callback_type": "failure", "callback_vars": {"langsmith_project": "proj"}}, {"callback_name": "langfuse", "callback_vars": {"not_a_callback_var": "x"}}, "langfuse", 7, {"callback_vars": {}}, {"callback_name": "langfuse", "callback_vars": {"langfuse_host": "os.environ/LANGFUSE_HOST"}}], "callback_settings": {"success_callback": ["datadog"]}}"#,
    );
    let (log, _guard) = capture_warnings();
    let masked = resolve(&metadata).unwrap();
    assert_eq!(
        masked,
        resolved([&[], &["langsmith"], &[]], &[("langsmith_project", "proj")])
    );
    let warning = "Ignoring invalid team-level read callback metadata: ";
    assert_eq!(warned(&log, warning), 3);
}

#[test]
fn fold_callback_entries() {
    let metadata = json(
        r#"{"logging": [{"callback_name": "langfuse", "callback_type": "success", "callback_vars": {"langfuse_host": "https://lf.example", "newrelic_region": "eu"}}, {"callback_name": "datadog", "callback_type": "failure", "callback_vars": {"dd_site": "datadoghq.eu"}}, {"callback_name": "langsmith", "callback_vars": {"langsmith_project": "first", "signoz_ingestion_endpoint": "https://signoz.example"}}, {"callback_name": "arize", "callback_type": null, "callback_vars": {}}, {"callback_name": "langfuse", "callback_type": "success_and_failure", "callback_vars": {}}, {"callback_name": "langsmith", "callback_type": "success", "callback_vars": {"langsmith_project": "second"}}]}"#,
    );
    let (log, _guard) = capture_warnings();
    let masked = resolve(&metadata).unwrap();
    assert_eq!(
        masked,
        resolved(
            [
                &["langfuse", "langsmith", "arize"],
                &["datadog", "langsmith", "arize", "langfuse"],
                &["langsmith", "arize", "langfuse"],
            ],
            &[
                ("dd_site", "datadoghq.eu"),
                ("langfuse_host", "https://lf.example"),
                ("langsmith_project", "second")
            ],
        )
    );
    assert_eq!(warned(&log, "WARN"), 0);
}

#[rstest]
#[case::unknown_variable(r#"{"callback_vars": {"not_a_callback_var": "x"}}"#)]
#[case::callbacks_not_a_list(r#"{"success_callback": "langfuse"}"#)]
#[case::variable_not_text(r#"{"callback_vars": {"langfuse_host": 7}}"#)]
#[case::sensitive_variable_not_text(r#"{"callback_vars": {"langfuse_secret_key": 7}}"#)]
fn invalid_legacy_raises(#[case] settings: &str) {
    let metadata = json(&format!(r#"{{"callback_settings": {settings}}}"#));
    let error = resolve(&metadata).unwrap_err();
    assert!(matches!(error, TeamCallbackError::CallbackSettings { .. }));
    assert_eq!(error.failure(), None);
}

#[rstest]
#[case::admin_not_found(StatusCode::NOT_FOUND)]
#[case::admin_bad_request(StatusCode::BAD_REQUEST)]
fn admin_unknown_team_status(#[case] status: StatusCode) {
    let error = unknown_team_error("team-9", Some(UserRole::ProxyAdmin), status);
    assert_eq!(
        error.failure(),
        Some(HttpFailure {
            status,
            detail: ErrorDetail::Error {
                error: "Team id = team-9 does not exist.".to_owned()
            }
        })
    );
}

#[rstest]
#[case::no_role(None)]
#[case::admin_viewer(Some(UserRole::ProxyAdminViewer))]
#[case::org_admin(Some(UserRole::OrgAdmin))]
#[case::internal_user(Some(UserRole::InternalUser))]
fn nonadmin_unknown_team_403(#[case] role: Option<UserRole>) {
    let error = unknown_team_error("team-9", role, StatusCode::NOT_FOUND);
    assert_eq!(
        error.failure(),
        Some(HttpFailure {
            status: StatusCode::FORBIDDEN,
            detail: ErrorDetail::Text("You do not have access to this team".to_owned())
        })
    );
}

#[derive(Default)]
struct HeldTasks(Mutex<Vec<AuditCallbackFuture>>);

impl AuditTaskSpawner for HeldTasks {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

impl HeldTasks {
    fn take(&self) -> Vec<AuditCallbackFuture> {
        mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(NOW).unwrap()
    }
}

struct FailingWriter;

#[async_trait]
impl TeamAuditWriter for FailingWriter {
    async fn write(&self, _record: AuditLogRecord) -> Result<(), AuditRepositoryError> {
        Err(AuditRepositoryError::Unavailable)
    }
}

#[derive(Default)]
struct StoredRows(Mutex<Vec<AuditLogRecord>>);

#[async_trait]
impl AuditLogRepository for StoredRows {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

struct NoAuditCallbacks;

impl AuditCallbackRegistry for NoAuditCallbacks {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

fn audit_settings(store_audit_logs: Option<bool>, env: Option<bool>) -> ConfiguredAuditSettings {
    let config = Settings {
        store_audit_logs,
        store_audit_logs_env: env,
        audit_log_callbacks: Vec::new(),
        s3_audit_callback_params: None,
    };
    ConfiguredAuditSettings {
        config,
        callbacks: Vec::new(),
    }
}

struct AuditFixture {
    tasks: Arc<HeldTasks>,
    rows: Arc<StoredRows>,
    audit: TeamCallbackAudit,
}

fn audit_fixture(
    settings: ConfiguredAuditSettings,
    failing: bool,
    admin: Option<&str>,
) -> AuditFixture {
    let tasks = Arc::new(HeldTasks::default());
    let rows = Arc::new(StoredRows::default());
    let shared_settings = Arc::new(settings);
    let spawner: Arc<dyn AuditTaskSpawner> = Arc::<HeldTasks>::clone(&tasks);
    let new_id: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| AUDIT_ID.to_owned());
    let sink = AuditEventSink {
        settings: Arc::<ConfiguredAuditSettings>::clone(&shared_settings),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(NoAuditCallbacks),
            Arc::clone(&spawner),
        )),
        repository: Arc::<StoredRows>::clone(&rows),
        clock: Arc::new(FixedClock),
        new_id: Arc::clone(&new_id),
    };
    let writer: Arc<dyn TeamAuditWriter> = if failing {
        Arc::new(FailingWriter)
    } else {
        Arc::new(sink)
    };
    let audit = TeamCallbackAudit {
        settings: shared_settings,
        clock: Arc::new(FixedClock),
        new_id,
        writer,
        spawner,
        litellm_proxy_admin_name: admin.map(str::to_owned),
    };
    AuditFixture { tasks, rows, audit }
}

const BEFORE: &str = r#"{"owner": "ops", "logging": [{"callback_name": "langsmith", "callback_vars": {"langsmith_project": "proj", "gcs_path_service_account": "/keys/sa.json"}}]}"#;
const AFTER: &str = r#"{"owner": "ops", "logging": [], "callback_settings": {"callback_vars": {"langfuse_host": "https://lf.example"}}}"#;

fn emit(fixture: &AuditFixture, caller: &CallerIdentity, header: Option<&str>) {
    let (before, after) = (json(BEFORE), json(AFTER));
    let change = TeamCallbackChange {
        team_id: "team-1",
        before_metadata: &before,
        after_metadata: &after,
        caller,
        litellm_changed_by: header,
    };
    fixture.audit.emit(&change).unwrap();
}

#[rstest]
#[case::store_false(Some(false), None)]
#[case::store_false_env_true(Some(false), Some(true))]
#[case::env_false(None, Some(false))]
fn audit_disabled_no_task(#[case] store: Option<bool>, #[case] env: Option<bool>) {
    let fixture = audit_fixture(audit_settings(store, env), false, Some("admin"));
    emit(&fixture, &CallerIdentity::default(), Some("ops-bot"));
    assert!(fixture.tasks.take().is_empty());
}

#[rstest]
#[case::header_wins(Some("ops-bot"), Some("user-1"), Some("admin"), Some("ops-bot"))]
#[case::empty_header_user(Some(""), Some("user-1"), Some("admin"), Some("user-1"))]
#[case::empty_user_admin(None, Some(""), Some("admin"), Some("admin"))]
#[case::nobody(None, None, None, None)]
#[tokio::test]
async fn audit_identity_and_json(
    #[case] header: Option<&str>,
    #[case] user_id: Option<&str>,
    #[case] admin: Option<&str>,
    #[case] changed_by: Option<&str>,
) {
    let fixture = audit_fixture(audit_settings(None, None), false, admin);
    let caller = CallerIdentity {
        user_id: user_id.map(str::to_owned),
        api_key: Some(SecretString::from("hashed-key")),
        metadata: None,
        team_metadata: None,
    };
    emit(&fixture, &caller, header);
    let mut tasks = fixture.tasks.take();
    let Some(task) = tasks.pop() else {
        panic!("one audit task is scheduled");
    };
    assert!(tasks.is_empty());
    task.await;
    let rows = fixture.rows.0.lock().unwrap().clone();
    let [row] = rows.as_slice() else {
        panic!("one audit row is written");
    };
    assert_eq!(row.id, AUDIT_ID);
    assert_eq!(row.updated_at.to_rfc3339(), NOW);
    assert_eq!(row.changed_by.as_deref(), changed_by);
    assert_eq!(
        row.changed_by_api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("hashed-key".to_owned())
    );
    assert_eq!(
        (row.action, row.table_name, row.object_id.as_str()),
        (AuditAction::Updated, AuditTable::Team, "team-1")
    );
    let before: JsonValue = serde_json::from_str(r#"{"metadata": {"owner": "ops", "logging": [{"callback_name": "langsmith", "callback_vars": {"langsmith_project": "***REDACTED***", "gcs_path_service_account": "***REDACTED***"}}]}}"#).unwrap();
    let after: JsonValue = serde_json::from_str(r#"{"metadata": {"owner": "ops", "logging": [], "callback_settings": {"callback_vars": {"langfuse_host": "***REDACTED***"}}}}"#).unwrap();
    assert_eq!(row.before_value, Some(before));
    assert_eq!(row.updated_values, Some(after));
}

#[tokio::test]
async fn audit_task_failure_warns() {
    let fixture = audit_fixture(audit_settings(Some(true), None), true, None);
    let (log, _guard) = capture_warnings();
    emit(&fixture, &CallerIdentity::default(), None);
    let mut tasks = fixture.tasks.take();
    let Some(task) = tasks.pop() else {
        panic!("one audit task is scheduled");
    };
    assert!(tasks.is_empty());
    task.await;
    let warning = "Failed to write team-callback audit log: no database connection";
    assert_eq!(warned(&log, warning), 1);
}

#[tokio::test]
async fn audit_cancel_no_warning() {
    let fixture = audit_fixture(audit_settings(Some(true), None), true, None);
    let (log, _guard) = capture_warnings();
    emit(&fixture, &CallerIdentity::default(), None);
    let tasks = fixture.tasks.take();
    assert_eq!(tasks.len(), 1);
    drop(tasks);
    yield_now().await;
    assert_eq!(warned(&log, "WARN"), 0);
    assert!(fixture.rows.0.lock().unwrap().is_empty());
}
