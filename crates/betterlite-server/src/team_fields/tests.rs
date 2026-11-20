#![cfg(test)]

use std::fs::write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_fields::schema::MetadataSchemaField;
use betterlite_config::sections::team_fields::{
    DEFAULT_UNAVAILABLE_MESSAGE, Settings, ValidationTimeout,
};
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::Manager;
use deadpool_postgres::Pool;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Serialize;
use tempfile::TempDir;
use tokio_postgres::NoTls;

use crate::python_hooks::PythonHooks;
use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;

use super::install::{InstallError, install_team_metadata};
use super::metadata::{
    DEFAULT_REJECTED_MESSAGE, MetadataRequest, TeamMetadataOperation, TeamMetadataRequester,
    TeamMetadataValidationPayload, build_payload,
};
use super::python_validator::ValidatorFailure;
use super::rows::{
    CallerRole, GeneralSettingsMap, LogValue, RefreshError, SettingsMapError, TeamEditAccess,
    TeamObjectCache, caller_edit_access, refresh_cached_team, sanitize_for_log,
};
use super::tables::{team_table, team_transaction};
use super::validation::{
    NOT_ASYNC_MESSAGE, TeamMetadataError, TeamMetadataSchemaRegistry, TeamMetadataValidator,
    TeamMetadataValidatorRegistry, validate_team_metadata_if_configured,
};

fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn serialize<T: Serialize>(value: &T) -> JsonData {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn request_for(metadata: &JsonData) -> MetadataRequest<'_> {
    MetadataRequest {
        operation: TeamMetadataOperation::Create,
        metadata,
        existing_metadata: None,
        team_id: Some("team-1"),
        team_alias: Some("Alias"),
        requester: TeamMetadataRequester {
            user_id: Some("u1".to_owned()),
            user_email: Some("u@example.com".to_owned()),
            user_role: Some("team_admin".to_owned()),
        },
    }
}

fn payload_for(
    metadata: &JsonData,
    existing_metadata: Option<&JsonData>,
) -> TeamMetadataValidationPayload {
    build_payload(MetadataRequest {
        operation: TeamMetadataOperation::Update,
        metadata,
        existing_metadata,
        team_id: Some("team-1"),
        team_alias: Some("Alias"),
        requester: TeamMetadataRequester {
            user_id: Some("u1".to_owned()),
            user_email: None,
            user_role: None,
        },
    })
}

struct FakeValidator {
    asynchronous: bool,
    calls: AtomicUsize,
}

impl FakeValidator {
    const fn new(asynchronous: bool) -> Self {
        Self {
            asynchronous,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TeamMetadataValidator for FakeValidator {
    fn is_async(&self) -> bool {
        self.asynchronous
    }

    async fn invoke(
        &self,
        _payload: &TeamMetadataValidationPayload,
        _timeout: ValidationTimeout,
    ) -> Result<JsonData, ValidatorFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(data(r#"{"valid": true}"#))
    }
}

#[test]
fn registry_empty_replace_clear() {
    let registry = TeamMetadataValidatorRegistry::default();
    assert!(registry.get().is_none());
    let first: Arc<dyn TeamMetadataValidator> = Arc::new(FakeValidator::new(true));
    registry.set(Some(Arc::clone(&first)));
    let current = registry.get().unwrap();
    assert!(Arc::ptr_eq(&current, &first));
    assert!(current.is_async());
    registry.set(None);
    assert!(registry.get().is_none());
}

fn schema_field(key: &str, label: Option<&str>) -> TeamMetadataFieldSchema {
    TeamMetadataFieldSchema {
        key: key.to_owned(),
        label: label.map(str::to_owned),
    }
}

#[tokio::test]
async fn schema_order() {
    let settings = Settings {
        schema: vec![
            MetadataSchemaField {
                key: "second".to_owned(),
                label: None,
            },
            MetadataSchemaField {
                key: "first".to_owned(),
                label: Some("First".to_owned()),
            },
        ],
        ..Settings::default()
    };
    let hooks = PythonHooks::new(None);
    let validators = TeamMetadataValidatorRegistry::default();
    let schemas = TeamMetadataSchemaRegistry::default();
    install_team_metadata(&hooks, &settings, &validators, &schemas)
        .await
        .unwrap();
    assert!(validators.get().is_none());
    assert_eq!(
        schemas.get(),
        vec![
            schema_field("second", None),
            schema_field("first", Some("First")),
        ]
    );
}

#[test]
fn metadata_dict_only() {
    let object = data(r#"{"owner": "env"}"#);
    let kept = payload_for(&object, Some(&object));
    assert_eq!(
        kept.metadata.get("owner").cloned(),
        Some(JsonData::String("env".to_owned()))
    );
    assert_eq!(kept.existing_metadata, Some(kept.metadata.clone()));
    assert_eq!(payload_for(&object, None).existing_metadata, None);
    for other in [
        data("7"),
        data("\"text\""),
        data("true"),
        data("null"),
        data("[\"owner\"]"),
    ] {
        let payload = payload_for(&other, Some(&other));
        assert!(payload.metadata.is_empty());
        assert_eq!(payload.existing_metadata, None);
    }
}

#[test]
fn requester_null_role() {
    let metadata = data("{}");
    let mut payload = payload_for(&metadata, None);
    payload.requester.user_role = None;
    let serialized = serialize(&payload);
    let JsonData::Object(fields) = &serialized else {
        panic!("expected a JSON object");
    };
    let JsonData::Object(requester) = &fields["requester"] else {
        panic!("expected a JSON object");
    };
    assert_eq!(requester.get("user_role").cloned(), Some(JsonData::Null));
    payload.requester.user_role = Some("team_admin".to_owned());
    let serialized = serialize(&payload);
    let JsonData::Object(fields) = &serialized else {
        panic!("expected a JSON object");
    };
    let JsonData::Object(requester) = &fields["requester"] else {
        panic!("expected a JSON object");
    };
    assert_eq!(
        requester.get("user_role").cloned(),
        Some(JsonData::String("team_admin".to_owned()))
    );
}

#[tokio::test]
async fn validator_absent() {
    let registry = TeamMetadataValidatorRegistry::default();
    let metadata = data(r#"{"owner": "env"}"#);
    validate_team_metadata_if_configured(request_for(&metadata), &registry, &Settings::default())
        .await
        .unwrap();
}

struct Workspace {
    directory: TempDir,
    config_path: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.yaml");
        write(&config_path, b"model_list: []\n").unwrap();
        Self {
            directory,
            config_path,
        }
    }

    fn module(&self, name: &str, source: &str) -> String {
        write(self.directory.path().join(format!("{name}.py")), source).unwrap();
        name.to_owned()
    }
}

fn hook_settings(module: &str, function: &str) -> Settings {
    Settings {
        validator: Some(format!("{module}.{function}")),
        ..Settings::default()
    }
}

async fn install_hook(settings: Settings, hooks: &PythonHooks) -> TeamMetadataValidatorRegistry {
    let validators = TeamMetadataValidatorRegistry::default();
    let schemas = TeamMetadataSchemaRegistry::default();
    install_team_metadata(hooks, &settings, &validators, &schemas)
        .await
        .unwrap();
    validators
}

async fn check(
    validators: &TeamMetadataValidatorRegistry,
    metadata: &JsonData,
    settings: &Settings,
) -> Result<(), TeamMetadataError> {
    validate_team_metadata_if_configured(request_for(metadata), validators, settings).await
}

const SIMPLE_ACCEPT: &str = "async def validate(payload):\n    return {\"valid\": True}\n";

const CALLABLE_OBJECT: &str = "class Validator:\n    async def __call__(self, payload):\n        return {\"valid\": True}\n\nvalidator = Validator()\n";

const SYNC_ACCEPT: &str = "def validate(payload):\n    return {\"valid\": True}\n";

const ACCEPTS: &str = "async def validate(payload):\n    metadata = payload.metadata\n    existing = payload.existing_metadata\n    requester = payload.requester\n    if payload.operation != \"create\":\n        return {\"valid\": False, \"error_message\": \"operation \" + str(payload.operation)}\n    if metadata.get(\"owner\") != \"env\":\n        return {\"valid\": False, \"error_message\": \"metadata \" + repr(metadata)}\n    if existing != {\"owner\": \"old\"}:\n        return {\"valid\": False, \"error_message\": \"existing \" + repr(existing)}\n    if (payload.team_id, payload.team_alias) != (\"team-1\", \"Alias\"):\n        return {\"valid\": False, \"error_message\": \"team \" + repr((payload.team_id, payload.team_alias))}\n    if (requester.user_id, requester.user_email, requester.user_role) != (\"u1\", \"u@example.com\", \"team_admin\"):\n        return {\"valid\": False, \"error_message\": \"requester \" + repr((requester.user_id, requester.user_email, requester.user_role))}\n    return {\"valid\": True, \"error_message\": \"ignored\"}\n";

const REJECTS: &str = "async def validate(payload):\n    return {\"valid\": False, \"error_message\": \"alias must be set\"}\n";

const REJECTS_EMPTY: &str =
    "async def validate(payload):\n    return {\"valid\": False, \"error_message\": \"\"}\n";

const SLEEPS: &str = "import asyncio\n\nasync def validate(payload):\n    await asyncio.sleep(30)\n    return {\"valid\": True}\n";

const RAISES: &str =
    "async def validate(payload):\n    raise ValueError(\"database exploded secret-token-123\")\n";

const MALFORMED: &str = "async def bad_type(payload):\n    return {\"valid\": \"yes\"}\n\nasync def not_an_object(payload):\n    return \"ok\"\n";

const CHECKS_METADATA: &str = "ALLOWED = (\"env\",)\n\nasync def validate(payload):\n    value = payload.metadata.get(\"owner\")\n    if value is None:\n        return {\"valid\": False, \"error_message\": \"owner is required\"}\n    if not isinstance(value, str):\n        return {\"valid\": False, \"error_message\": \"owner must be a string\"}\n    if value not in ALLOWED:\n        return {\"valid\": False, \"error_message\": \"owner must be one of env\"}\n    return {\"valid\": True}\n";

const OTHER_FUNCTION: &str = "def other():\n    return None\n";

#[tokio::test]
async fn async_function() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_async_function", SIMPLE_ACCEPT);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    assert!(validators.get().unwrap().is_async());
    check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn async_callable_object() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_callable_object", CALLABLE_OBJECT);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validator"), &hooks).await;
    assert!(validators.get().unwrap().is_async());
    check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn sync_callable_500() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_sync_callable", SYNC_ACCEPT);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    assert!(!validators.get().unwrap().is_async());
    let error = check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(error, TeamMetadataError::NotAsync);
    assert_eq!(error.status(), 500);
    assert_eq!(error.message(), NOT_ASYNC_MESSAGE);
}

#[tokio::test]
async fn validator_accepts() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_accepts", ACCEPTS);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let metadata = data(r#"{"owner": "env"}"#);
    let existing = data(r#"{"owner": "old"}"#);
    let request = MetadataRequest {
        operation: TeamMetadataOperation::Create,
        metadata: &metadata,
        existing_metadata: Some(&existing),
        team_id: Some("team-1"),
        team_alias: Some("Alias"),
        requester: TeamMetadataRequester {
            user_id: Some("u1".to_owned()),
            user_email: Some("u@example.com".to_owned()),
            user_role: Some("team_admin".to_owned()),
        },
    };
    validate_team_metadata_if_configured(request, &validators, &Settings::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn rejection_message() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_rejects", REJECTS);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let error = check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Rejected {
            message: "alias must be set".to_owned(),
        }
    );
    assert_eq!(error.status(), 400);
}

#[tokio::test]
async fn rejection_empty_message() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_rejects_empty", REJECTS_EMPTY);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let error = check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Rejected {
            message: DEFAULT_REJECTED_MESSAGE.to_owned(),
        }
    );
    assert_eq!(error.status(), 400);
}

#[tokio::test]
async fn validator_timeout() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_sleeps", SLEEPS);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let settings = Settings {
        timeout: ValidationTimeout::Finite(Duration::from_millis(100)),
        unavailable_message: "validator timed out".to_owned(),
        ..hook_settings(&module, "validate")
    };
    let validators = install_hook(settings.clone(), &hooks).await;
    let error = check(&validators, &data("{}"), &settings)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Unavailable {
            message: "validator timed out".to_owned(),
        }
    );
    assert_eq!(error.status(), 503);
}

#[tokio::test]
async fn validator_raised_503() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_raises", RAISES);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let error = check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Unavailable {
            message: DEFAULT_UNAVAILABLE_MESSAGE.to_owned(),
        }
    );
    assert_eq!(error.status(), 503);
}

#[tokio::test]
async fn malformed_result_503() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_malformed", MALFORMED);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let first = install_hook(hook_settings(&module, "bad_type"), &hooks).await;
    let error = check(&first, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Unavailable {
            message: DEFAULT_UNAVAILABLE_MESSAGE.to_owned(),
        }
    );
    let second = install_hook(hook_settings(&module, "not_an_object"), &hooks).await;
    let error = check(&second, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TeamMetadataError::Unavailable {
            message: DEFAULT_UNAVAILABLE_MESSAGE.to_owned(),
        }
    );
    assert_eq!(error.status(), 503);
}

#[tokio::test]
async fn no_exception_leak() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_no_leak", RAISES);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let error = check(&validators, &data("{}"), &Settings::default())
        .await
        .unwrap_err();
    assert_eq!(error.message(), DEFAULT_UNAVAILABLE_MESSAGE);
    assert!(!error.message().contains("secret-token-123"));
    assert!(!error.message().contains("database exploded"));
    assert!(!format!("{error:?}").contains("secret-token-123"));
}

#[tokio::test]
async fn metadata_without_license() {
    // Nothing in this path checks premium_user, so the callback and its errors run license free.
    let workspace = Workspace::new();
    let module = workspace.module("b801_required_fields", CHECKS_METADATA);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = install_hook(hook_settings(&module, "validate"), &hooks).await;
    let settings = Settings::default();
    let missing = check(&validators, &data("{}"), &settings)
        .await
        .unwrap_err();
    assert_eq!(
        missing,
        TeamMetadataError::Rejected {
            message: "owner is required".to_owned(),
        }
    );
    let wrong_type = check(&validators, &data(r#"{"owner": 7}"#), &settings)
        .await
        .unwrap_err();
    assert_eq!(
        wrong_type,
        TeamMetadataError::Rejected {
            message: "owner must be a string".to_owned(),
        }
    );
    let disallowed = check(&validators, &data(r#"{"owner": "prod"}"#), &settings)
        .await
        .unwrap_err();
    assert_eq!(
        disallowed,
        TeamMetadataError::Rejected {
            message: "owner must be one of env".to_owned(),
        }
    );
    check(&validators, &data(r#"{"owner": "env"}"#), &settings)
        .await
        .unwrap();
}

#[tokio::test]
async fn missing_validator_bridge() {
    let workspace = Workspace::new();
    let module = workspace.module("b801_bridge", OTHER_FUNCTION);
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    let validators = TeamMetadataValidatorRegistry::default();
    let schemas = TeamMetadataSchemaRegistry::default();
    let missing_module = hook_settings("b801_absent_module", "validate");
    let error = install_team_metadata(&hooks, &missing_module, &validators, &schemas)
        .await
        .unwrap_err();
    assert!(matches!(error, InstallError::Load { .. }));
    assert!(validators.get().is_none());
    let missing_attribute = hook_settings(&module, "validate");
    let error = install_team_metadata(&hooks, &missing_attribute, &validators, &schemas)
        .await
        .unwrap_err();
    assert!(matches!(error, InstallError::Load { .. }));
    assert!(validators.get().is_none());
}

#[test]
fn sanitize_crlf() {
    assert_eq!(sanitize_for_log(LogValue::Display("a\rb")), "ab");
    assert_eq!(sanitize_for_log(LogValue::Display("a\nb")), "ab");
    assert_eq!(sanitize_for_log(LogValue::Display("a\r\nb")), "ab");
    assert_eq!(
        sanitize_for_log(LogValue::Display("plain text")),
        "plain text"
    );
}

struct NewlineDebug;

impl std::fmt::Debug for NewlineDebug {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("first\nsecond\rthird")
    }
}

#[test]
fn sanitize_repr_fallback() {
    assert_eq!(
        sanitize_for_log(LogValue::Debug(&NewlineDebug)),
        "firstsecondthird"
    );
}

#[derive(Default)]
struct RecordingCache {
    stored: Mutex<Vec<(String, LiteLLMTeamTableCachedObj)>>,
}

impl RecordingCache {
    fn stored(&self) -> Vec<(String, LiteLLMTeamTableCachedObj)> {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TeamObjectCache for RecordingCache {
    async fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError> {
        team.last_refreshed_at = Some(42.5);
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((team_id.to_owned(), team.clone()));
        Ok(())
    }
}

struct FailingCache;

#[async_trait]
impl TeamObjectCache for FailingCache {
    async fn store_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError> {
        Err(InvalidationError::SpendOutOfRange(Decimal::ZERO))
    }
}

#[tokio::test]
async fn cache_row_validated() {
    let record = TeamRecord::parse(
        br#"{"team_id": "team-1", "team_alias": "Alias", "metadata": "{\"owner\": \"env\"}"}"#,
    )
    .unwrap()
    .unwrap();
    let cache = RecordingCache::default();
    refresh_cached_team(&record, &cache).await.unwrap();
    let stored = cache.stored();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, "team-1");
    assert!(stored[0].1.last_refreshed_at.is_some());
    assert_eq!(
        stored[0].1.team.fields.base.team_alias,
        Some("Alias".to_owned())
    );
    assert_eq!(
        stored[0]
            .1
            .team
            .fields
            .base
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("owner").cloned()),
        Some(JsonData::String("env".to_owned()))
    );
}

#[tokio::test]
async fn cache_refresh_error() {
    let undecodable = TeamRecord::parse(br#"{"team_alias": "Alias"}"#)
        .unwrap()
        .unwrap();
    let error = refresh_cached_team(&undecodable, &RecordingCache::default())
        .await
        .unwrap_err();
    assert!(matches!(error, RefreshError::Row(_)));
    let record = TeamRecord::parse(br#"{"team_id": "team-1"}"#)
        .unwrap()
        .unwrap();
    let error = refresh_cached_team(&record, &FailingCache)
        .await
        .unwrap_err();
    assert!(matches!(error, RefreshError::Cache(_)));
}

fn settings_map(text: &str) -> GeneralSettingsMap {
    let value: JsonValue = serde_json::from_str(text).unwrap();
    GeneralSettingsMap::from_value(&value).unwrap()
}

#[test]
fn unrestricted_edit() {
    let settings = settings_map("{}");
    assert_eq!(
        caller_edit_access(Some(CallerRole::ProxyAdmin), &settings),
        TeamEditAccess::Unrestricted
    );
    assert_eq!(
        caller_edit_access(Some(CallerRole::OrgAdmin), &settings),
        TeamEditAccess::Unrestricted
    );
}

#[test]
fn disabled_team_edit() {
    let settings = settings_map("{}");
    assert_eq!(
        caller_edit_access(Some(CallerRole::TeamAdmin), &settings),
        TeamEditAccess::TeamAdminDisabled
    );
    let empty = settings_map(r#"{"team_admin_editable_team_fields": []}"#);
    assert_eq!(
        caller_edit_access(Some(CallerRole::TeamAdmin), &empty),
        TeamEditAccess::TeamAdminDisabled
    );
    let unsupported = settings_map(r#"{"team_admin_editable_team_fields": ["projects"]}"#);
    assert_eq!(
        caller_edit_access(Some(CallerRole::TeamAdmin), &unsupported),
        TeamEditAccess::TeamAdminDisabled
    );
}

#[test]
fn sorted_edit_fields() {
    let settings =
        settings_map(r#"{"team_admin_editable_team_fields": ["tpm_limit", "max_budget"]}"#);
    assert_eq!(
        caller_edit_access(Some(CallerRole::TeamAdmin), &settings),
        TeamEditAccess::TeamAdmin {
            editable_fields: vec!["max_budget".to_owned(), "tpm_limit".to_owned()],
        }
    );
}

#[test]
fn no_role_edit() {
    assert_eq!(
        caller_edit_access(None, &settings_map("{}")),
        TeamEditAccess::None
    );
}

#[test]
fn settings_map_error() {
    let list: JsonValue = serde_json::from_str("[]").unwrap();
    assert_eq!(GeneralSettingsMap::from_value(&list), Err(SettingsMapError));
    let schema = settings_map(r#"{"team_admin_editable_team_fields": ["tpm_limit"]}"#);
    assert_eq!(schema.entries.len(), 1);
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    members_with_roles JSONB
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    user_email TEXT
);
"#;

const INSERT_TEAM_SQL: &str =
    r#"INSERT INTO "LiteLLM_TeamTable" (team_id, members_with_roles) VALUES ($1, '[]'::jsonb)"#;
const DELETE_TEAM_SQL: &str = r#"DELETE FROM "LiteLLM_TeamTable" WHERE team_id = $1"#;
const INSERT_USER_SQL: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email) VALUES ($1, $2)"#;
const COUNT_TEAM_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_TeamTable" WHERE team_id = $1"#;
const COUNT_USER_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_UserTable" WHERE user_id = $1"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["team-fields-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

async fn count(pool: &Pool, sql: &str, key: &str) -> i64 {
    let client = pool.get().await.unwrap();
    let row = client.query_one(sql, &[&key]).await.unwrap();
    row.get(0)
}

#[tokio::test]
async fn transaction_accessor() {
    let (fixture, pool) = database().await;
    let seeded = pool.get().await.unwrap();
    seeded.execute(INSERT_TEAM_SQL, &[&"team-1"]).await.unwrap();
    drop(seeded);
    let mut client = pool.get().await.unwrap();
    let session = client.transaction().await.unwrap();
    let transaction = team_transaction(&session);
    let members = transaction
        .members_with_roles("team-1")
        .await
        .unwrap()
        .unwrap();
    assert!(members.is_empty());
    assert_eq!(
        transaction
            .execute_raw(INSERT_USER_SQL, &[&"user-1", &"u@example.com"])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        transaction
            .execute_raw(DELETE_TEAM_SQL, &[&"team-1"])
            .await
            .unwrap(),
        1
    );
    assert_eq!(transaction.team_row("team-1").await.unwrap(), None);
    assert_eq!(
        transaction.members_with_roles("team-1").await.unwrap(),
        None
    );
    transaction
        .execute_raw(INSERT_TEAM_SQL, &[&"team-2"])
        .await
        .unwrap();
    let record = transaction.team_row("team-2").await.unwrap().unwrap();
    assert_eq!(
        record.fields.get("team_id").cloned(),
        Some(JsonData::String("team-2".to_owned()))
    );
    let user = transaction
        .user_table()
        .find_by_id("user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.user_id, "user-1");
    assert_eq!(user.user_email.as_deref(), Some("u@example.com"));
    session.rollback().await.unwrap();
    drop(client);
    assert_eq!(count(&pool, COUNT_TEAM_SQL, "team-1").await, 1);
    assert_eq!(count(&pool, COUNT_TEAM_SQL, "team-2").await, 0);
    assert_eq!(count(&pool, COUNT_USER_SQL, "user-1").await, 0);
    pool.close();
    fixture.close_async().await.unwrap();
}

#[tokio::test]
async fn team_table_selects_the_read_store() {
    let (fixture, pool) = database().await;
    let seeded = pool.get().await.unwrap();
    seeded.execute(INSERT_TEAM_SQL, &[&"team-1"]).await.unwrap();
    drop(seeded);
    let writer = PostgresStore {
        pool: Some(pool.clone()),
    };
    let missing = PostgresStore { pool: None };
    let availability = Availability::default();
    availability
        .reader_unavailable
        .store(false, Ordering::SeqCst);
    let client = PrismaClient {
        writer: &writer,
        reader: &missing,
        availability: &availability,
    };
    let teams = team_table(Some(client)).unwrap();
    assert!(matches!(
        teams.table().find_unique("team-1").await,
        Err(TeamQueryError::Database { .. })
    ));
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let teams = team_table(Some(client)).unwrap();
    let record = teams.table().find_unique("team-1").await.unwrap().unwrap();
    assert_eq!(
        record.fields.get("team_id").cloned(),
        Some(JsonData::String("team-1".to_owned()))
    );
    assert!(matches!(team_table(None), Err(DbError::NoDbConnected)));
    pool.close();
    fixture.close_async().await.unwrap();
}
