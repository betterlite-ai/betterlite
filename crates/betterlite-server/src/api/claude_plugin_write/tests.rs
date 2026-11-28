#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::claude_plugin_policy::DB_NOT_CONNECTED;
use betterlite_core::api::claude_plugin_write::StoredPlugin;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_policy_resources::ADD_CLAUDE_CODE_PLUGIN_TABLE;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tower::ServiceExt as _;

use super::store::{
    ClaudePluginStore, NewPlugin, PluginChanges, PluginCreateError, PostgresPluginStore,
};
use super::{ClaudePluginWriteState, mount};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::mcp::outbound_credentials::WallClock;
use crate::routes::router;
use crate::routes::tests::state_for;

const CREATE_FULL_BODY: &str = r#"{"name":"oracle-plugin","source":{"source":"github","repo":"org/oracle-plugin"},"version":"2.1.0","description":"A plugin","author":{"name":"Ada"},"homepage":"https://example.com/plugin","keywords":["one","two"],"category":"dev","domain":"Productivity","namespace":"workflows"}"#;
const CREATED_FULL: &str = r#"{"status":"success","action":"created","plugin":{"id":"plugin-oracle-plugin","name":"oracle-plugin","version":"2.1.0","description":"A plugin","source":{"source":"github","repo":"org/oracle-plugin"},"enabled":true}}"#;
const CREATE_FULL_MANIFEST: &str = r#"{"name":"oracle-plugin","source":{"source":"github","repo":"org/oracle-plugin"},"version":"2.1.0","description":"A plugin","author":{"name":"Ada"},"homepage":"https://example.com/plugin","keywords":["one","two"],"category":"dev","domain":"Productivity","namespace":"workflows"}"#;

const CREATE_MINIMAL_BODY: &str = r#"{"name":"minimal-plugin","source":{"source":"git-subdir","url":"https://github.com/org/repo.git","path":"plugins/minimal"}}"#;
const CREATED_MINIMAL: &str = r#"{"status":"success","action":"created","plugin":{"id":"plugin-minimal-plugin","name":"minimal-plugin","version":"1.0.0","description":null,"source":{"source":"git-subdir","url":"https://github.com/org/repo.git","path":"plugins/minimal"},"enabled":true}}"#;
const CREATE_MINIMAL_MANIFEST: &str = r#"{"name":"minimal-plugin","source":{"source":"git-subdir","url":"https://github.com/org/repo.git","path":"plugins/minimal"},"version":"1.0.0"}"#;

const CREATE_INVALID_NAME: &str =
    r#"{"name":"Bad_Name","source":{"source":"github","repo":"org/bad"}}"#;
const INVALID_NAME_RESPONSE: &str = r#"{"detail":{"error":"Plugin name must be kebab-case (lowercase letters, numbers, hyphens)"}}"#;

const CONFLICT_BODY: &str =
    r#"{"name":"taken-plugin","source":{"source":"github","repo":"org/taken"}}"#;
const CONFLICT_RESPONSE: &str = r#"{"detail":{"error":"A skill named 'taken-plugin' already exists. Update the existing skill instead of adding it again."}}"#;

const ADMIN_ONLY_RESPONSE: &str =
    r#"{"detail":{"error":"Only proxy admins may modify the Claude Code plugin marketplace."}}"#;

const UPDATE_BODY: &str = r#"{"source":{"source":"archive","url":"https://bucket.example.com/plugin.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"version":"2.0.0","description":"replaced"}"#;
const UPDATED_RESPONSE: &str = r#"{"status":"success","action":"updated","plugin":{"id":"plugin-update-plugin","name":"update-plugin","version":"2.0.0","description":"replaced","source":{"source":"archive","url":"https://bucket.example.com/plugin.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"enabled":true}}"#;
const UPDATED_MANIFEST: &str = r#"{"name":"update-plugin","source":{"source":"archive","url":"https://bucket.example.com/plugin.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},"version":"2.0.0","description":"replaced"}"#;
const UPDATE_MISSING_RESPONSE: &str = r#"{"detail":{"error":"Plugin 'missing-plugin' not found"}}"#;
const UPDATE_INVALID_SOURCE: &str = r#"{"source":{"source":"github"}}"#;
const UPDATE_INVALID_SOURCE_RESPONSE: &str =
    r#"{"detail":{"error":"GitHub source must include 'repo' field (e.g., 'org/repo')"}}"#;

const DISABLED_RESPONSE: &str =
    r#"{"status":"success","message":"Plugin 'toggle-plugin' disabled"}"#;
const ENABLED_RESPONSE: &str = r#"{"status":"success","message":"Plugin 'toggle-plugin' enabled"}"#;
const ENABLE_MISSING_RESPONSE: &str = r#"{"detail":{"error":"Plugin 'absent-plugin' not found"}}"#;

const DELETE_CREATE_BODY: &str =
    r#"{"name":"delete-plugin","source":{"source":"url","url":"https://github.com/org/repo.git"}}"#;
const DELETE_MISSING_RESPONSE: &str = r#"{"detail":{"error":"Plugin 'absent-plugin' not found"}}"#;
const DELETED_RESPONSE: &str = r#"{"status":"success","message":"Plugin 'delete-plugin' deleted"}"#;

const ROUTE_CREATE: &str =
    r#"{"name":"route-plugin","source":{"source":"github","repo":"org/route-plugin"}}"#;
const ROUTE_UPDATE: &str = r#"{"source":{"source":"github","repo":"org/route-plugin"}}"#;

#[derive(Default)]
struct FakeState {
    rows: BTreeMap<String, StoredPlugin>,
    writes: Vec<NewPlugin>,
    changes: Vec<PluginChanges>,
    toggles: Vec<(String, bool)>,
    deleted: Vec<String>,
}

struct FakeStore {
    state: Mutex<FakeState>,
    unique_violation: AtomicBool,
    fail_find: AtomicBool,
    fail_write: AtomicBool,
    vanish_on_update: AtomicBool,
}

impl FakeStore {
    fn new() -> Self {
        Self {
            state: Mutex::new(FakeState::default()),
            unique_violation: AtomicBool::new(false),
            fail_find: AtomicBool::new(false),
            fail_write: AtomicBool::new(false),
            vanish_on_update: AtomicBool::new(false),
        }
    }

    fn seed(&self, name: &str, enabled: bool) {
        let mut guard = self.state.lock().expect("the fake store");
        guard.rows.insert(
            name.to_owned(),
            StoredPlugin {
                id: format!("plugin-{name}"),
                name: name.to_owned(),
                version: Some("1.0.0".to_owned()),
                description: Some("seeded".to_owned()),
                enabled,
            },
        );
        drop(guard);
    }

    fn row(&self, name: &str) -> Option<StoredPlugin> {
        let guard = self.state.lock().expect("the fake store");
        guard.rows.get(name).cloned()
    }
}

#[async_trait]
impl ClaudePluginStore for FakeStore {
    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPlugin>, TableError> {
        if self.fail_find.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        let guard = self.state.lock().expect("the fake store");
        Ok(guard.rows.get(plugin_name).cloned())
    }

    async fn create(&self, write: NewPlugin) -> Result<StoredPlugin, PluginCreateError> {
        if self.unique_violation.load(Ordering::SeqCst) {
            return Err(PluginCreateError::NameTaken);
        }
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(PluginCreateError::Table(TableError::MissingRow));
        }
        let mut guard = self.state.lock().expect("the fake store");
        let stored = StoredPlugin {
            id: format!("plugin-{}", write.name),
            name: write.name.clone(),
            version: write.version.clone(),
            description: write.description.clone(),
            enabled: write.enabled,
        };
        guard.rows.insert(stored.name.clone(), stored.clone());
        guard.writes.push(write);
        drop(guard);
        Ok(stored)
    }

    async fn update(&self, write: PluginChanges) -> Result<Option<StoredPlugin>, TableError> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        if self.vanish_on_update.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut guard = self.state.lock().expect("the fake store");
        let Some(existing) = guard.rows.get_mut(&write.name) else {
            return Ok(None);
        };
        existing.version = write.version.clone();
        existing.description = write.description.clone();
        let stored = existing.clone();
        guard.changes.push(write);
        drop(guard);
        Ok(Some(stored))
    }

    async fn set_enabled(
        &self,
        plugin_name: &str,
        enabled: bool,
        _updated_at: NaiveDateTime,
    ) -> Result<(), TableError> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        let mut guard = self.state.lock().expect("the fake store");
        if let Some(existing) = guard.rows.get_mut(plugin_name) {
            existing.enabled = enabled;
        }
        guard.toggles.push((plugin_name.to_owned(), enabled));
        drop(guard);
        Ok(())
    }

    async fn delete(&self, plugin_name: &str) -> Result<(), TableError> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        let mut guard = self.state.lock().expect("the fake store");
        guard.rows.remove(plugin_name);
        guard.deleted.push(plugin_name.to_owned());
        drop(guard);
        Ok(())
    }
}

struct FixedClock(DateTime<Utc>);

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0)
        .single()
        .expect("the fixed instant")
}

// deadpool builds its pool lazily, so a well-formed URL proves the store without a connection
#[tokio::test]
async fn database_store_resolves_the_configured_url() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "general_settings:\n  master_key: oracle-master\n  database_url: postgresql://plugin:secret@127.0.0.1:5432/plugins\n",
    )
    .expect("the test config");
    let state = state_for(&config);
    assert!(super::database_store(&state).is_some());
}

fn caller_from(identity: &str) -> AuthContext {
    let principal: Principal = serde_json::from_str(identity).expect("the test principal");
    AuthContext { principal }
}

fn master() -> AuthContext {
    master_caller()
}

fn plain_user() -> AuthContext {
    caller_from(r#"{"principal_type":"human","subject":"plain-key","auth_method":"api_key"}"#)
}

fn role_admin() -> AuthContext {
    caller_from(
        r#"{"principal_type":"human","subject":"admin-key","user":{"id":"key-owner"},"roles":["platform_admin"],"auth_method":"api_key"}"#,
    )
}

struct Reply {
    status: StatusCode,
    text: String,
}

async fn read(application: Router, request: Request<Body>) -> Reply {
    let response = application
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body");
    Reply {
        status,
        text: String::from_utf8(bytes.to_vec()).expect("the response is UTF-8"),
    }
}

fn body_of(body: Option<&str>) -> Body {
    body.map_or_else(Body::empty, |text| Body::from(text.to_owned()))
}

fn app(store: Arc<FakeStore>) -> Router {
    mount(ClaudePluginWriteState {
        store: Some(store),
        clock: Arc::new(FixedClock(fixed_now())),
        limits: ApiLimits::default(),
    })
}

async fn call(
    store: &Arc<FakeStore>,
    caller: &AuthContext,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Reply {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .extension(caller.clone())
        .body(body_of(body))
        .expect("the request builds");
    read(app(Arc::clone(store)), request).await
}

fn booted(master_key: &str) -> Router {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!("general_settings:\n  master_key: {master_key}\n"))
    .expect("the test config");
    router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    )
}

async fn booted_call(
    application: Router,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    body: Option<&str>,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    let request = builder.body(body_of(body)).expect("the request builds");
    read(application, request).await
}

#[rstest]
#[case::full(CREATE_FULL_BODY, CREATED_FULL, Some(CREATE_FULL_MANIFEST))]
#[case::minimal(CREATE_MINIMAL_BODY, CREATED_MINIMAL, Some(CREATE_MINIMAL_MANIFEST))]
#[case::invalid_name(CREATE_INVALID_NAME, INVALID_NAME_RESPONSE, None)]
#[tokio::test]
async fn plugin_create(#[case] body: &str, #[case] expected: &str, #[case] manifest: Option<&str>) {
    let store = Arc::new(FakeStore::new());
    let reply = call(
        &store,
        &master(),
        "POST",
        "/claude-code/plugins",
        Some(body),
    )
    .await;
    let status = if manifest.is_some() {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    assert_eq!(reply.status, status);
    assert_eq!(reply.text, expected);
    let guard = store.state.lock().expect("the fake store");
    let Some(expected_manifest) = manifest else {
        assert!(guard.writes.is_empty());
        return;
    };
    assert_eq!(guard.writes.len(), 1);
    let write = guard.writes.first().expect("the recorded write");
    assert_eq!(write.manifest_json, expected_manifest);
    assert!(write.enabled);
    assert_eq!(write.created_at, fixed_now().naive_utc());
    assert_eq!(write.updated_at, write.created_at);
    assert_eq!(write.created_by.as_deref(), Some("default_user_id"));
    assert_eq!(guard.rows.len(), 1);
}

#[tokio::test]
async fn non_admin_is_forbidden() {
    let store = Arc::new(FakeStore::new());
    let reply = call(
        &store,
        &plain_user(),
        "POST",
        "/claude-code/plugins",
        Some(CREATE_MINIMAL_BODY),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.text, ADMIN_ONLY_RESPONSE);
    assert!(
        store
            .state
            .lock()
            .expect("the fake store")
            .writes
            .is_empty()
    );
}

#[tokio::test]
async fn role_admin_records_the_key_owner() {
    let store = Arc::new(FakeStore::new());
    let reply = call(
        &store,
        &role_admin(),
        "POST",
        "/claude-code/plugins",
        Some(CREATE_MINIMAL_BODY),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let guard = store.state.lock().expect("the fake store");
    let write = guard.writes.first().expect("the recorded write");
    assert_eq!(write.created_by.as_deref(), Some("key-owner"));
    drop(guard);
}

#[rstest]
#[case::existing_row(false)]
#[case::unique_violation(true)]
#[tokio::test]
async fn plugin_name_conflict(#[case] race: bool) {
    let store = Arc::new(FakeStore::new());
    if race {
        store.unique_violation.store(true, Ordering::SeqCst);
    } else {
        store.seed("taken-plugin", true);
    }
    let reply = call(
        &store,
        &master(),
        "POST",
        "/claude-code/plugins",
        Some(CONFLICT_BODY),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.text, CONFLICT_RESPONSE);
    assert!(
        store
            .state
            .lock()
            .expect("the fake store")
            .writes
            .is_empty()
    );
}

#[rstest]
#[case::replaces(
    "update-plugin",
    UPDATE_BODY,
    StatusCode::OK,
    UPDATED_RESPONSE,
    Some(UPDATED_MANIFEST)
)]
#[case::missing(
    "missing-plugin",
    UPDATE_BODY,
    StatusCode::NOT_FOUND,
    UPDATE_MISSING_RESPONSE,
    None
)]
#[case::invalid_source(
    "update-plugin",
    UPDATE_INVALID_SOURCE,
    StatusCode::BAD_REQUEST,
    UPDATE_INVALID_SOURCE_RESPONSE,
    None
)]
#[tokio::test]
async fn plugin_update_source(
    #[case] name: &str,
    #[case] body: &str,
    #[case] status: StatusCode,
    #[case] expected: &str,
    #[case] manifest: Option<&str>,
) {
    let store = Arc::new(FakeStore::new());
    store.seed("update-plugin", true);
    let path = format!("/claude-code/plugins/{name}");
    let reply = call(&store, &master(), "PUT", &path, Some(body)).await;
    assert_eq!(reply.status, status);
    assert_eq!(reply.text, expected);
    let guard = store.state.lock().expect("the fake store");
    let Some(expected_manifest) = manifest else {
        assert!(guard.changes.is_empty());
        return;
    };
    assert_eq!(guard.changes.len(), 1);
    let change = guard.changes.first().expect("the recorded change");
    assert_eq!(change.name, "update-plugin");
    assert_eq!(change.manifest_json, expected_manifest);
    assert_eq!(change.updated_at, fixed_now().naive_utc());
    let row = guard.rows.get("update-plugin").expect("the updated row");
    assert_eq!(row.description.as_deref(), Some("replaced"));
    assert_eq!(row.version.as_deref(), Some("2.0.0"));
    assert!(row.enabled);
    drop(guard);
}

// the row can disappear between the presence check and the conditional update, which LiteLLM answers 404
#[tokio::test]
async fn update_that_loses_the_row_answers_not_found() {
    let store = Arc::new(FakeStore::new());
    store.seed("update-plugin", true);
    store.vanish_on_update.store(true, Ordering::SeqCst);
    let reply = call(
        &store,
        &master(),
        "PUT",
        "/claude-code/plugins/update-plugin",
        Some(UPDATE_BODY),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.text,
        r#"{"detail":{"error":"Plugin 'update-plugin' not found"}}"#
    );
}

async fn toggle_call(store: &Arc<FakeStore>, plugin_name: &str, action: &str) -> Reply {
    let path = format!("/claude-code/plugins/{plugin_name}/{action}");
    call(store, &master(), "POST", &path, None).await
}

fn toggles_of(store: &Arc<FakeStore>) -> Vec<(String, bool)> {
    store.state.lock().expect("the fake store").toggles.clone()
}

#[tokio::test]
async fn plugin_disable_enable() {
    let store = Arc::new(FakeStore::new());
    store.seed("toggle-plugin", true);
    let disabled = toggle_call(&store, "toggle-plugin", "disable").await;
    assert_eq!(disabled.status, StatusCode::OK);
    assert_eq!(disabled.text, DISABLED_RESPONSE);
    assert!(!store.row("toggle-plugin").expect("the row").enabled);
    let enabled = toggle_call(&store, "toggle-plugin", "enable").await;
    assert_eq!(enabled.status, StatusCode::OK);
    assert_eq!(enabled.text, ENABLED_RESPONSE);
    assert!(store.row("toggle-plugin").expect("the row").enabled);
    let missing = toggle_call(&store, "absent-plugin", "enable").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.text, ENABLE_MISSING_RESPONSE);
    assert_eq!(
        toggles_of(&store),
        vec![
            ("toggle-plugin".to_owned(), false),
            ("toggle-plugin".to_owned(), true)
        ]
    );
}

async fn delete_call(store: &Arc<FakeStore>, plugin_name: &str) -> Reply {
    let path = format!("/claude-code/plugins/{plugin_name}");
    call(store, &master(), "DELETE", &path, None).await
}

#[tokio::test]
async fn plugin_delete_missing() {
    let store = Arc::new(FakeStore::new());
    let missing = delete_call(&store, "absent-plugin").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.text, DELETE_MISSING_RESPONSE);
    let created = call(
        &store,
        &master(),
        "POST",
        "/claude-code/plugins",
        Some(DELETE_CREATE_BODY),
    )
    .await;
    assert_eq!(created.status, StatusCode::OK);
    let deleted = delete_call(&store, "delete-plugin").await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.text, DELETED_RESPONSE);
    assert!(store.row("delete-plugin").is_none());
    let again = delete_call(&store, "delete-plugin").await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(
        store.state.lock().expect("the fake store").deleted,
        vec!["delete-plugin".to_owned()]
    );
}

struct FailureCase {
    method: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    seed: bool,
    fail_find: bool,
    expected: &'static str,
}

#[rstest]
#[case::registration(FailureCase {
    method: "POST",
    path: "/claude-code/plugins",
    body: Some(CONFLICT_BODY),
    seed: false,
    fail_find: false,
    expected: "Registration failed: statement returned no row",
})]
#[case::registration_find(FailureCase {
    method: "POST",
    path: "/claude-code/plugins",
    body: Some(CONFLICT_BODY),
    seed: false,
    fail_find: true,
    expected: "Registration failed: statement returned no row",
})]
#[case::update(FailureCase {
    method: "PUT",
    path: "/claude-code/plugins/taken-plugin",
    body: Some(ROUTE_UPDATE),
    seed: true,
    fail_find: false,
    expected: "Update failed: statement returned no row",
})]
#[case::update_find(FailureCase {
    method: "PUT",
    path: "/claude-code/plugins/taken-plugin",
    body: Some(ROUTE_UPDATE),
    seed: true,
    fail_find: true,
    expected: "Update failed: statement returned no row",
})]
#[case::enable(FailureCase {
    method: "POST",
    path: "/claude-code/plugins/taken-plugin/enable",
    body: None,
    seed: true,
    fail_find: false,
    expected: "statement returned no row",
})]
#[case::enable_find(FailureCase {
    method: "POST",
    path: "/claude-code/plugins/taken-plugin/enable",
    body: None,
    seed: true,
    fail_find: true,
    expected: "statement returned no row",
})]
#[case::disable(FailureCase {
    method: "POST",
    path: "/claude-code/plugins/taken-plugin/disable",
    body: None,
    seed: true,
    fail_find: false,
    expected: "statement returned no row",
})]
#[case::delete(FailureCase {
    method: "DELETE",
    path: "/claude-code/plugins/taken-plugin",
    body: None,
    seed: true,
    fail_find: false,
    expected: "statement returned no row",
})]
#[tokio::test]
async fn store_failures_use_the_handler_prefix(#[case] case: FailureCase) {
    let store = Arc::new(FakeStore::new());
    if case.seed {
        store.seed("taken-plugin", true);
    }
    if case.fail_find {
        store.fail_find.store(true, Ordering::SeqCst);
    } else {
        store.fail_write.store(true, Ordering::SeqCst);
    }
    let reply = call(&store, &master(), case.method, case.path, case.body).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text,
        format!(r#"{{"detail":{{"error":"{}"}}}}"#, case.expected)
    );
}

#[rstest]
#[case::create("POST", "/claude-code/plugins", Some(ROUTE_CREATE))]
#[case::update("PUT", "/claude-code/plugins/route-plugin", Some(ROUTE_UPDATE))]
#[case::enable("POST", "/claude-code/plugins/route-plugin/enable", None)]
#[case::disable("POST", "/claude-code/plugins/route-plugin/disable", None)]
#[case::delete("DELETE", "/claude-code/plugins/route-plugin", None)]
#[tokio::test]
async fn route_aliases(#[case] method: &str, #[case] path: &str, #[case] body: Option<&str>) {
    let reply = booted_call(
        booted("oracle-master"),
        method,
        path,
        Some("Bearer oracle-master"),
        body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text,
        format!(r#"{{"detail":{{"error":"{DB_NOT_CONNECTED}"}}}}"#)
    );
}

#[rstest]
#[case::create("POST", "/claude-code/plugins")]
#[case::item("PUT", "/claude-code/plugins/auth-plugin")]
#[case::enable("POST", "/claude-code/plugins/auth-plugin/enable")]
#[case::disable("POST", "/claude-code/plugins/auth-plugin/disable")]
#[case::delete("DELETE", "/claude-code/plugins/auth-plugin")]
#[tokio::test]
async fn auth_missing(#[case] method: &str, #[case] path: &str) {
    let reply = booted_call(booted("oracle-master"), method, path, None, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        reply.text,
        r#"{"error":{"message":"the API key is not valid","type":"auth_error","param":"None","code":"401"}}"#
    );
}

#[tokio::test]
async fn auth_invalid() {
    let reply = booted_call(
        booted("oracle-master"),
        "POST",
        "/claude-code/plugins",
        Some("Bearer sk-not-the-master"),
        Some(ROUTE_CREATE),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.text,
        r#"{"error":{"message":"the API key is not valid","type":"no_db_connection","param":null,"code":"400"}}"#
    );
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    let state = ClaudePluginWriteState {
        store: Some(Arc::new(FakeStore::new())),
        clock: Arc::new(FixedClock(fixed_now())),
        limits: ApiLimits {
            max_json_bytes: 1,
            max_event_bytes: 1024 * 1024,
            max_items: 64 * 1024,
        },
    };
    let request = Request::builder()
        .method("POST")
        .uri("/claude-code/plugins")
        .header("content-type", "application/json")
        .extension(master())
        .body(body_of(Some(CREATE_MINIMAL_BODY)))
        .expect("the request builds");
    let reply = read(mount(state), request).await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        reply.text,
        format!(
            r#"{{"error":{{"message":"the request JSON body is {} bytes; the maximum is 1","type":"invalid_request_error","param":"body","code":"request_too_large"}}}}"#,
            CREATE_MINIMAL_BODY.len()
        )
    );
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, PostgresPluginStore) {
    let table = from_utf8(ADD_CLAUDE_CODE_PLUGIN_TABLE.bytes)
        .expect("the plugin table migration is UTF-8")
        .to_owned();
    let url = Environment::parse_from(["claude-plugin-write-tests"])
        .url
        .map(SecretString::from);
    let statements = vec![table];
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the postgres fixture");
    let settings = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let manager = Manager::new(settings, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the postgres pool");
    let store = PostgresPluginStore::new(Arc::new(PostgresStore {
        pool: Some(pool.clone()),
    }));
    (fixture, pool, store)
}

fn round_trip_write(now: NaiveDateTime) -> NewPlugin {
    NewPlugin {
        name: "round-trip-plugin".to_owned(),
        version: Some("1.0.0".to_owned()),
        description: Some("first".to_owned()),
        manifest_json: r#"{"name":"round-trip-plugin"}"#.to_owned(),
        enabled: true,
        created_at: now,
        updated_at: now,
        created_by: Some("alice".to_owned()),
    }
}

fn round_trip_changes(changed_at: NaiveDateTime) -> PluginChanges {
    PluginChanges {
        name: "round-trip-plugin".to_owned(),
        version: None,
        description: Some("second".to_owned()),
        manifest_json: r#"{"name":"round-trip-plugin","version":null}"#.to_owned(),
        updated_at: changed_at,
    }
}

fn missing_changes(now: NaiveDateTime) -> PluginChanges {
    PluginChanges {
        name: "absent-plugin".to_owned(),
        version: None,
        description: None,
        manifest_json: "{}".to_owned(),
        updated_at: now,
    }
}

async fn assert_created_row(pool: &Pool, now: NaiveDateTime) {
    let client = pool.get().await.expect("the pooled client");
    let row = client
        .query_one(
            r#"SELECT manifest_json, files_json, created_by, created_at, updated_at FROM "LiteLLM_ClaudeCodePluginTable" WHERE name = $1"#,
            &[&"round-trip-plugin"],
        )
        .await
        .expect("the stored row");
    let manifest: Option<String> = row.get("manifest_json");
    let files: Option<String> = row.get("files_json");
    let creator: Option<String> = row.get("created_by");
    let created_at: Option<NaiveDateTime> = row.get("created_at");
    let updated_at: Option<NaiveDateTime> = row.get("updated_at");
    assert_eq!(manifest.as_deref(), Some(r#"{"name":"round-trip-plugin"}"#));
    assert_eq!(files.as_deref(), Some("{}"));
    assert_eq!(creator.as_deref(), Some("alice"));
    assert_eq!(created_at, Some(now));
    assert_eq!(updated_at, Some(now));
    drop(client);
}

async fn assert_updated_row(pool: &Pool, changed_at: NaiveDateTime) {
    let client = pool.get().await.expect("the pooled client");
    let row = client
        .query_one(
            r#"SELECT manifest_json, files_json, updated_at FROM "LiteLLM_ClaudeCodePluginTable" WHERE name = $1"#,
            &[&"round-trip-plugin"],
        )
        .await
        .expect("the updated row");
    let manifest: Option<String> = row.get("manifest_json");
    let files: Option<String> = row.get("files_json");
    let updated_at: Option<NaiveDateTime> = row.get("updated_at");
    assert_eq!(
        manifest.as_deref(),
        Some(r#"{"name":"round-trip-plugin","version":null}"#)
    );
    assert_eq!(files.as_deref(), Some("{}"));
    assert_eq!(updated_at, Some(changed_at));
    drop(client);
}

async fn assert_toggled_row(pool: &Pool, toggled_at: NaiveDateTime) {
    let client = pool.get().await.expect("the pooled client");
    let row = client
        .query_one(
            r#"SELECT updated_at FROM "LiteLLM_ClaudeCodePluginTable" WHERE name = $1"#,
            &[&"round-trip-plugin"],
        )
        .await
        .expect("the toggled row");
    let updated_at: Option<NaiveDateTime> = row.get("updated_at");
    assert_eq!(updated_at, Some(toggled_at));
    drop(client);
}

async fn create_round_trip(store: &PostgresPluginStore, now: NaiveDateTime) {
    let created = store
        .create(round_trip_write(now))
        .await
        .expect("the created row");
    assert_eq!(created.name, "round-trip-plugin");
    assert!(created.enabled);
    assert_eq!(created.version.as_deref(), Some("1.0.0"));
    assert!(!created.id.is_empty());
    let duplicate = store.create(round_trip_write(now)).await;
    assert!(matches!(duplicate, Err(PluginCreateError::NameTaken)));
}

async fn update_round_trip(store: &PostgresPluginStore, changed_at: NaiveDateTime) {
    let changed = store
        .update(round_trip_changes(changed_at))
        .await
        .expect("the update")
        .expect("the updated row");
    assert_eq!(changed.version, None);
    assert_eq!(changed.description.as_deref(), Some("second"));
}

async fn stored_plugin(store: &PostgresPluginStore) -> StoredPlugin {
    store
        .find_by_name("round-trip-plugin")
        .await
        .expect("the find")
        .expect("the stored row")
}

async fn toggle_round_trip(store: &PostgresPluginStore, toggled_at: NaiveDateTime) {
    store
        .set_enabled("round-trip-plugin", false, toggled_at)
        .await
        .expect("the toggle");
    assert!(!stored_plugin(store).await.enabled);
}

async fn delete_round_trip(store: &PostgresPluginStore, now: NaiveDateTime) {
    store.delete("round-trip-plugin").await.expect("the delete");
    assert!(
        store
            .find_by_name("round-trip-plugin")
            .await
            .expect("the find")
            .is_none()
    );
    assert!(
        store
            .update(missing_changes(now))
            .await
            .expect("the update")
            .is_none()
    );
}

#[tokio::test]
async fn store_round_trip() {
    let (fixture, pool, store) = database().await;
    let now = fixed_now().naive_utc();
    let changed_at = now + Duration::seconds(60);
    let toggled_at = changed_at + Duration::seconds(60);
    create_round_trip(&store, now).await;
    assert_created_row(&pool, now).await;
    update_round_trip(&store, changed_at).await;
    assert_updated_row(&pool, changed_at).await;
    toggle_round_trip(&store, toggled_at).await;
    assert_toggled_row(&pool, toggled_at).await;
    delete_round_trip(&store, now).await;
    pool.close();
    fixture.close_async().await.expect("the fixture closes");
}
