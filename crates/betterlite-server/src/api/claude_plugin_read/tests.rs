#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::claude_plugin_policy::{
    DB_NOT_CONNECTED, NameFilter, PluginWhere, PluginWhereBranch,
};
use betterlite_core::api::claude_plugin_read::{
    ClaudePluginReadInput, ClaudePluginReadOperation, PluginRow,
};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Duration, TimeZone as _, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use serde::Deserialize;
use tower::ServiceExt as _;

use super::flow::plugin_row;
use super::store::{ClaudePluginReadStore, StoredPluginRow};
use super::{ClaudePluginReadState, decode_claude_plugin_read, mount};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::routes::router;
use crate::routes::tests::state_for;

mod store;

const ALL_FILTER: PluginWhere = PluginWhere {
    enabled: None,
    any: None,
};
const ENABLED_FILTER: PluginWhere = PluginWhere {
    enabled: Some(true),
    any: None,
};

const ALPHA_MANIFEST: &str = r#"{"name":"plugin-alpha","source":{"source":"github","repo":"org/alpha"},"author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://example.com/alpha","keywords":["one","two"],"category":"dev","domain":"Productivity","namespace":"workflows"}"#;
const BETA_MANIFEST: &str =
    r#"{"name":"plugin-beta","source":{"source":"url","url":"https://example.com/beta.git"}}"#;
const GAMMA_MANIFEST: &str =
    r#"{"name":"plugin-gamma","source":{"source":"github","repo":"org/gamma","branch":"main"}}"#;

const ALPHA_ITEM: &str = r#"{"id":"id-plugin-alpha","name":"plugin-alpha","version":"1.0.0","description":"seeded","source":{"source":"github","repo":"org/alpha"},"author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://example.com/alpha","keywords":["one","two"],"category":"dev","domain":"Productivity","namespace":"workflows","enabled":true,"created_at":"2026-03-15T10:00:00+00:00","updated_at":"2026-03-15T10:00:00+00:00"}"#;
const BETA_ITEM: &str = r#"{"id":"id-plugin-beta","name":"plugin-beta","version":"1.0.0","description":"seeded","source":{"source":"url","url":"https://example.com/beta.git"},"author":null,"homepage":null,"keywords":null,"category":null,"domain":null,"namespace":null,"enabled":true,"created_at":"2026-03-15T10:01:15.250000+00:00","updated_at":"2026-03-15T10:01:15.250000+00:00"}"#;
const GAMMA_ITEM: &str = r#"{"id":"id-plugin-gamma","name":"plugin-gamma","version":"1.0.0","description":"seeded","source":{"source":"github","repo":"org/gamma","branch":"main"},"author":null,"homepage":null,"keywords":null,"category":null,"domain":null,"namespace":null,"enabled":false,"created_at":"2026-03-15T10:02:00+00:00","updated_at":"2026-03-15T10:02:00+00:00"}"#;

const FULL_GET_MANIFEST: &str = r#"{"name":"plugin-get","source":{"source":"github","repo":"org/get"},"author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://example.com/get","keywords":["one","two"],"category":"dev","domain":"Productivity","namespace":"workflows"}"#;
const RAW_GET_MANIFEST: &str = r#"{"source":{"source":"github","repo":"org/raw","custom":1},"author":{"name":"Ada","unknown":true},"homepage":123,"keywords":"raw","category":["a"]}"#;
const NULL_FIELDS_MANIFEST: &str = r#"{"source":{"source":"github","repo":"org/null"},"author":null,"homepage":null,"keywords":null,"category":null,"domain":null,"namespace":null}"#;
const NO_EMAIL_MANIFEST: &str = r#"{"author":{"name":"Ada","extra":"ignored"}}"#;
const NULL_EMAIL_MANIFEST: &str = r#"{"author":{"name":"Ada","email":null}}"#;

const FULL_GET_BODY: &str = r#"{"id":"id-plugin-get","name":"plugin-get","version":"1.0.0","description":"seeded","source":{"source":"github","repo":"org/get"},"author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://example.com/get","keywords":["one","two"],"category":"dev","enabled":true,"created_at":"2026-03-15T10:03:00+00:00","updated_at":"2026-03-15T10:03:00+00:00","created_by":"alice"}"#;
const MINIMAL_GET_BODY: &str = r#"{"id":"id-plugin-get","name":"plugin-get","version":"1.0.0","description":"seeded","source":null,"author":null,"homepage":null,"keywords":null,"category":null,"enabled":true,"created_at":"2026-03-15T10:03:00+00:00","updated_at":"2026-03-15T10:03:00+00:00","created_by":"alice"}"#;
const RAW_GET_BODY: &str = r#"{"id":"id-plugin-get","name":"plugin-get","version":"1.0.0","description":"seeded","source":{"source":"github","repo":"org/raw","custom":1},"author":{"name":"Ada","unknown":true},"homepage":123,"keywords":"raw","category":["a"],"enabled":true,"created_at":"2026-03-15T10:03:00+00:00","updated_at":"2026-03-15T10:03:00+00:00","created_by":"alice"}"#;

const PRIVATE_DETAIL: &str = r#"{"id":"id-private-plugin","name":"private-plugin","version":"1.0.0","description":"seeded","source":null,"author":null,"homepage":null,"keywords":null,"category":null,"enabled":false,"created_at":"2026-03-15T11:01:00+00:00","updated_at":"2026-03-15T11:01:00+00:00","created_by":"alice"}"#;
const NOT_GRANTED_DETAIL: &str =
    r#"{"detail":{"error":"Plugin 'private-plugin' is not granted to this key"}}"#;
const NOT_FOUND_DETAIL: &str = r#"{"detail":{"error":"Plugin 'plugin-absent' not found"}}"#;
const INVALID_MANIFEST_DETAIL: &str =
    r#"{"detail":{"error":"the stored plugin manifest is not valid JSON"}}"#;
const SHAPE_DETAIL: &str = r#"{"detail":{"error":"the stored plugin manifest is not an object"}}"#;
const STORE_FAILURE_DETAIL: &str = r#"{"detail":{"error":"statement returned no row"}}"#;

const KEY_PRIVATE_GRANT: &str = r#"{"token":"sk-private-grant","object_permission":{"object_permission_id":"perm-key-private","skills":["private-plugin"]}}"#;
const KEY_TEAM_GRANT: &str = r#"{"token":"sk-team-grant","object_permission_id":"perm-team","team_object_permission":{"object_permission_id":"perm-team-private","skills":["private-plugin"]}}"#;
const KEY_INTERSECTION: &str = r#"{"token":"sk-intersection","object_permission":{"object_permission_id":"perm-key-both","skills":["private-plugin","other-plugin"]},"team_object_permission":{"object_permission_id":"perm-team-both","skills":["private-plugin"]}}"#;
const KEY_DISJOINT: &str = r#"{"token":"sk-disjoint","object_permission":{"object_permission_id":"perm-key-disjoint","skills":["private-plugin"]},"team_object_permission":{"object_permission_id":"perm-team-disjoint","skills":["other-plugin"]}}"#;
const KEY_EMPTY_GRANTS: &str = r#"{"token":"sk-empty-grants","object_permission":{"object_permission_id":"perm-empty","skills":[]}}"#;
const KEY_PLAIN: &str = r#"{"token":"sk-plain"}"#;

#[derive(Default)]
struct FakeState {
    rows: Vec<StoredPluginRow>,
    filters: Vec<PluginWhere>,
}

struct FakeStore {
    state: Mutex<FakeState>,
    fail: AtomicBool,
    ignore_filters: AtomicBool,
}

impl FakeStore {
    fn new() -> Self {
        Self {
            state: Mutex::new(FakeState::default()),
            fail: AtomicBool::new(false),
            ignore_filters: AtomicBool::new(false),
        }
    }

    fn seed(&self, row: StoredPluginRow) {
        let mut guard = self.state.lock().expect("the fake store");
        guard.rows.push(row);
        drop(guard);
    }

    fn filters(&self) -> Vec<PluginWhere> {
        let guard = self.state.lock().expect("the fake store");
        guard.filters.clone()
    }
}

fn matches_filter(row: &StoredPluginRow, filter: &PluginWhere) -> bool {
    if let Some(branches) = filter.any.as_ref() {
        return branches.iter().any(|branch| match branch {
            PluginWhereBranch::Enabled { enabled } => row.enabled == *enabled,
            PluginWhereBranch::Name { name } => name.names.contains(&row.name),
        });
    }
    filter.enabled.is_none_or(|enabled| row.enabled == enabled)
}

#[async_trait]
impl ClaudePluginReadStore for FakeStore {
    async fn list(&self, filter: &PluginWhere) -> Result<Vec<StoredPluginRow>, TableError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        let mut guard = self.state.lock().expect("the fake store");
        guard.filters.push(filter.clone());
        let rows = if self.ignore_filters.load(Ordering::SeqCst) {
            guard.rows.clone()
        } else {
            guard
                .rows
                .iter()
                .filter(|row| matches_filter(row, filter))
                .cloned()
                .collect()
        };
        drop(guard);
        Ok(rows)
    }

    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPluginRow>, TableError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(TableError::MissingRow);
        }
        let guard = self.state.lock().expect("the fake store");
        Ok(guard
            .rows
            .iter()
            .find(|row| row.name == plugin_name)
            .cloned())
    }
}

fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, hour, minute, second)
        .single()
        .expect("the fixed instant")
}

fn beta_created() -> DateTime<Utc> {
    at(10, 1, 15)
        .checked_add_signed(Duration::microseconds(250_000))
        .expect("the fixed instant")
}

fn stored_row(name: &str, enabled: bool, created: DateTime<Utc>) -> StoredPluginRow {
    StoredPluginRow {
        id: format!("id-{name}"),
        name: name.to_owned(),
        version: Some("1.0.0".to_owned()),
        description: Some("seeded".to_owned()),
        manifest_json: None,
        enabled,
        created_at: Some(created),
        updated_at: Some(created),
        created_by: Some("alice".to_owned()),
    }
}

fn granted_filter(names: &[&str]) -> PluginWhere {
    PluginWhere {
        enabled: None,
        any: Some(vec![
            PluginWhereBranch::Enabled { enabled: true },
            PluginWhereBranch::Name {
                name: NameFilter {
                    names: names.iter().map(|name| (*name).to_owned()).collect(),
                },
            },
        ]),
    }
}

fn list_body(items: &[&str]) -> String {
    format!(
        r#"{{"plugins":[{}],"count":{}}}"#,
        items.join(","),
        items.len()
    )
}

fn default_item(source: &str, author: &str) -> String {
    format!(
        r#"{{"id":"id-plugin-default","name":"plugin-default","version":"1.0.0","description":"seeded","source":{source},"author":{author},"homepage":null,"keywords":null,"category":null,"domain":null,"namespace":null,"enabled":true,"created_at":"2026-03-15T12:02:00+00:00","updated_at":"2026-03-15T12:02:00+00:00"}}"#
    )
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

fn platform_viewer() -> AuthContext {
    caller_from(
        r#"{"principal_type":"human","subject":"viewer-key","user":{"id":"viewer"},"roles":["platform_viewer"],"auth_method":"api_key"}"#,
    )
}

fn key_caller(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs");
    let mut context =
        caller_from(r#"{"principal_type":"human","subject":"key-owner","auth_method":"api_key"}"#);
    context.principal.source_key = SourceKey(Some(key));
    context
}

struct Reply {
    status: StatusCode,
    text: String,
}

async fn send(application: Router, request: Request<Body>) -> Reply {
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

fn app(store: Arc<FakeStore>) -> Router {
    mount(ClaudePluginReadState {
        store: Some(store),
        limits: ApiLimits::default(),
    })
}

async fn call(store: &Arc<FakeStore>, caller: Option<&AuthContext>, path: &str) -> Reply {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(context) = caller {
        builder = builder.extension(context.clone());
    }
    let request = builder.body(Body::empty()).expect("the request builds");
    send(app(Arc::clone(store)), request).await
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

async fn booted_call(application: Router, path: &str, authorization: Option<&str>) -> Reply {
    let mut builder = Request::builder().method("GET").uri(path);
    if let Some(value) = authorization {
        builder = builder.header("authorization", value);
    }
    let request = builder.body(Body::empty()).expect("the request builds");
    send(application, request).await
}

#[derive(Deserialize)]
struct ListedBody {
    plugins: Vec<ListedPlugin>,
}

#[derive(Deserialize)]
struct ListedPlugin {
    name: String,
}

fn names(reply: &Reply) -> Vec<String> {
    let body: ListedBody = serde_json::from_str(&reply.text).expect("the body is a plugin list");
    body.plugins.into_iter().map(|plugin| plugin.name).collect()
}

fn pagination_store() -> Arc<FakeStore> {
    let store = Arc::new(FakeStore::new());
    let mut alpha = stored_row("plugin-alpha", true, at(10, 0, 0));
    alpha.manifest_json = Some(ALPHA_MANIFEST.to_owned());
    let mut beta = stored_row("plugin-beta", true, beta_created());
    beta.manifest_json = Some(BETA_MANIFEST.to_owned());
    let mut gamma = stored_row("plugin-gamma", false, at(10, 2, 0));
    gamma.manifest_json = Some(GAMMA_MANIFEST.to_owned());
    store.seed(alpha);
    store.seed(beta);
    store.seed(gamma);
    store
}

#[rstest]
#[case::default("", false, &[GAMMA_ITEM, BETA_ITEM, ALPHA_ITEM])]
#[case::enabled_true("?enabled_only=true", true, &[BETA_ITEM, ALPHA_ITEM])]
#[case::enabled_one("?enabled_only=1", true, &[BETA_ITEM, ALPHA_ITEM])]
#[case::enabled_upper("?enabled_only=TRUE", true, &[BETA_ITEM, ALPHA_ITEM])]
#[case::enabled_false("?enabled_only=false", false, &[GAMMA_ITEM, BETA_ITEM, ALPHA_ITEM])]
#[case::last_duplicate_wins(
    "?enabled_only=true&enabled_only=false",
    false,
    &[GAMMA_ITEM, BETA_ITEM, ALPHA_ITEM]
)]
#[tokio::test]
async fn plugin_list_pagination(
    #[case] query: &str,
    #[case] enabled_only: bool,
    #[case] items: &[&str],
) {
    let store = pagination_store();
    let path = format!("/claude-code/plugins{query}");
    let reply = call(&store, Some(&master()), &path).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text, list_body(items));
    let expected = if enabled_only {
        ENABLED_FILTER
    } else {
        ALL_FILTER
    };
    assert_eq!(store.filters(), vec![expected]);
}

#[rstest]
#[case::word("maybe")]
#[case::number("2")]
#[tokio::test]
async fn invalid_enabled_only_is_rejected(#[case] value: &str) {
    let store = pagination_store();
    let path = format!("/claude-code/plugins?enabled_only={value}");
    let reply = call(&store, Some(&master()), &path).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.text,
        r#"{"detail":[{"loc":["query","enabled_only"],"msg":"not a boolean","type":"bool_parsing"}]}"#
    );
    assert!(store.filters().is_empty());
}

#[rstest]
#[case::full(Some(FULL_GET_MANIFEST), FULL_GET_BODY)]
#[case::minimal(Some("{}"), MINIMAL_GET_BODY)]
#[case::stored_null(None, MINIMAL_GET_BODY)]
#[case::raw_types(Some(RAW_GET_MANIFEST), RAW_GET_BODY)]
#[tokio::test]
async fn plugin_get(#[case] manifest: Option<&str>, #[case] expected: &str) {
    let store = Arc::new(FakeStore::new());
    store.seed(StoredPluginRow {
        manifest_json: manifest.map(str::to_owned),
        ..stored_row("plugin-get", true, at(10, 3, 0))
    });
    let reply = call(&store, Some(&master()), "/claude-code/plugins/plugin-get").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text, expected);
}

async fn not_found_is_404(store: &Arc<FakeStore>) {
    let reply = call(store, Some(&master()), "/claude-code/plugins/plugin-absent").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.text, NOT_FOUND_DETAIL);
}

async fn broken_manifest_is_500(store: &Arc<FakeStore>) {
    let mut invalid = stored_row("plugin-broken", true, at(12, 4, 0));
    invalid.manifest_json = Some("{not json".to_owned());
    let mut shape = stored_row("plugin-shape", true, at(12, 5, 0));
    shape.manifest_json = Some("[]".to_owned());
    store.seed(invalid);
    store.seed(shape);
    let invalid_reply = call(store, Some(&master()), "/claude-code/plugins/plugin-broken").await;
    assert_eq!(invalid_reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(invalid_reply.text, INVALID_MANIFEST_DETAIL);
    let shape_reply = call(store, Some(&master()), "/claude-code/plugins/plugin-shape").await;
    assert_eq!(shape_reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(shape_reply.text, SHAPE_DETAIL);
}

async fn store_failure_is_500() {
    let store = Arc::new(FakeStore::new());
    store.seed(stored_row("plugin-stored", true, at(12, 6, 0)));
    store.fail.store(true, Ordering::SeqCst);
    let reply = call(
        &store,
        Some(&master()),
        "/claude-code/plugins/plugin-stored",
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.text, STORE_FAILURE_DETAIL);
}

#[tokio::test]
async fn plugin_not_found() {
    let store = Arc::new(FakeStore::new());
    not_found_is_404(&store).await;
    broken_manifest_is_500(&store).await;
    store_failure_is_500().await;
}

#[rstest]
#[case::source_list(r#"{"source":[]}"#, "source")]
#[case::source_text(r#"{"source":"github"}"#, "source")]
#[case::source_value(r#"{"source":{"repo":5}}"#, "source")]
#[case::source_null(r#"{"source":null}"#, "source")]
#[case::author_missing_name(r#"{"author":{"email":"ada@example.com"}}"#, "author")]
#[case::author_scalar(r#"{"author":"Ada"}"#, "author")]
#[case::author_name_type(r#"{"author":{"name":5}}"#, "author")]
#[case::author_email_type(r#"{"author":{"name":"Ada","email":5}}"#, "author")]
#[case::author_email_list(r#"{"author":{"name":"Ada","email":[]}}"#, "author")]
#[case::homepage_type(r#"{"homepage":5}"#, "homepage")]
#[case::homepage_bool(r#"{"homepage":true}"#, "homepage")]
#[case::keywords_type(r#"{"keywords":"one"}"#, "keywords")]
#[case::keywords_object(r#"{"keywords":{}}"#, "keywords")]
#[case::keyword_item(r#"{"keywords":["one",2]}"#, "keywords")]
#[case::category_type(r#"{"category":5}"#, "category")]
#[case::category_object(r#"{"category":{}}"#, "category")]
#[case::domain_type(r#"{"domain":[]}"#, "domain")]
#[case::namespace_type(r#"{"namespace":5}"#, "namespace")]
#[tokio::test]
async fn invalid_manifest_fields_answer_500(#[case] manifest: &str, #[case] field: &str) {
    let store = Arc::new(FakeStore::new());
    store.seed(StoredPluginRow {
        manifest_json: Some(manifest.to_owned()),
        ..stored_row("plugin-broken", true, at(12, 7, 0))
    });
    let reply = call(&store, Some(&master()), "/claude-code/plugins").await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text,
        format!(
            r#"{{"detail":{{"error":"the stored plugin manifest field '{field}' has an unexpected value"}}}}"#
        )
    );
}

#[rstest]
#[case::empty_object(Some("{}"), "{}", "null")]
#[case::stored_null(None, "{}", "null")]
#[case::null_fields(
    Some(NULL_FIELDS_MANIFEST),
    r#"{"source":"github","repo":"org/null"}"#,
    "null"
)]
#[case::author_without_email(Some(NO_EMAIL_MANIFEST), "{}", r#"{"name":"Ada","email":null}"#)]
#[case::author_null_email(Some(NULL_EMAIL_MANIFEST), "{}", r#"{"name":"Ada","email":null}"#)]
#[tokio::test]
async fn list_manifest_defaults(
    #[case] manifest: Option<&str>,
    #[case] source: &str,
    #[case] author: &str,
) {
    let store = Arc::new(FakeStore::new());
    store.seed(StoredPluginRow {
        manifest_json: manifest.map(str::to_owned),
        ..stored_row("plugin-default", true, at(12, 2, 0))
    });
    let reply = call(&store, Some(&master()), "/claude-code/plugins").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text, list_body(&[&default_item(source, author)]));
}

#[tokio::test]
async fn list_sorting() {
    let store = Arc::new(FakeStore::new());
    let mut first = stored_row("first-plugin", true, at(10, 0, 0));
    first.manifest_json = Some("{}".to_owned());
    let mut second = stored_row("second-plugin", true, at(10, 0, 0));
    second.manifest_json = Some("{}".to_owned());
    let mut latest = stored_row("latest-plugin", true, at(10, 5, 0));
    latest.manifest_json = Some("{}".to_owned());
    let mut undated = stored_row("undated-plugin", true, at(10, 9, 0));
    undated.created_at = None;
    undated.updated_at = None;
    undated.manifest_json = Some("{}".to_owned());
    store.seed(first);
    store.seed(second);
    store.seed(latest);
    store.seed(undated);
    let reply = call(&store, Some(&master()), "/claude-code/plugins").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        names(&reply),
        vec![
            "latest-plugin",
            "first-plugin",
            "second-plugin",
            "undated-plugin"
        ]
    );
}

fn visibility_store() -> Arc<FakeStore> {
    let store = Arc::new(FakeStore::new());
    let seeds = [
        (
            "public-plugin",
            true,
            0,
            r#"{"name":"public-plugin","source":{"source":"github","repo":"org/public"}}"#,
        ),
        ("private-plugin", false, 1, r#"{"name":"private-plugin"}"#),
        ("other-plugin", false, 2, r#"{"name":"other-plugin"}"#),
    ];
    for (name, enabled, hour, manifest) in seeds {
        let mut row = stored_row(name, enabled, at(11, hour, 0));
        row.manifest_json = Some(manifest.to_owned());
        store.seed(row);
    }
    store
}

async fn admin_visibility(store: &Arc<FakeStore>) {
    let listed = call(store, Some(&master()), "/claude-code/plugins").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        names(&listed),
        vec!["other-plugin", "private-plugin", "public-plugin"]
    );
    let detail = call(
        store,
        Some(&master()),
        "/claude-code/plugins/private-plugin",
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.text, PRIVATE_DETAIL);
    let admin = call(store, Some(&role_admin()), "/claude-code/plugins").await;
    assert_eq!(
        names(&admin),
        vec!["other-plugin", "private-plugin", "public-plugin"]
    );
    let viewer = call(store, Some(&platform_viewer()), "/claude-code/plugins").await;
    assert_eq!(names(&viewer), vec!["public-plugin"]);
    assert_eq!(
        store.filters(),
        vec![ALL_FILTER, ALL_FILTER, ENABLED_FILTER]
    );
}

async fn list_visibility(store: &Arc<FakeStore>) {
    let plain = call(store, Some(&plain_user()), "/claude-code/plugins").await;
    assert_eq!(names(&plain), vec!["public-plugin"]);
    for key in [KEY_PRIVATE_GRANT, KEY_TEAM_GRANT, KEY_INTERSECTION] {
        let granted = call(store, Some(&key_caller(key)), "/claude-code/plugins").await;
        assert_eq!(names(&granted), vec!["private-plugin", "public-plugin"]);
    }
    for key in [KEY_DISJOINT, KEY_EMPTY_GRANTS, KEY_PLAIN] {
        let ungranted = call(store, Some(&key_caller(key)), "/claude-code/plugins").await;
        assert_eq!(names(&ungranted), vec!["public-plugin"]);
    }
}

async fn get_visibility(store: &Arc<FakeStore>) {
    let user = plain_user();
    let private = "/claude-code/plugins/private-plugin";
    let hidden = call(store, Some(&user), private).await;
    assert_eq!(hidden.status, StatusCode::FORBIDDEN);
    assert_eq!(hidden.text, NOT_GRANTED_DETAIL);
    let public = call(store, Some(&user), "/claude-code/plugins/public-plugin").await;
    assert_eq!(public.status, StatusCode::OK);
    let key = key_caller(KEY_PRIVATE_GRANT);
    let granted = call(store, Some(&key), private).await;
    assert_eq!(granted.status, StatusCode::OK);
    assert_eq!(granted.text, PRIVATE_DETAIL);
    let team = call(store, Some(&key_caller(KEY_TEAM_GRANT)), private).await;
    assert_eq!(team.status, StatusCode::OK);
    let disjoint = call(store, Some(&key_caller(KEY_DISJOINT)), private).await;
    assert_eq!(disjoint.status, StatusCode::FORBIDDEN);
    assert_eq!(disjoint.text, NOT_GRANTED_DETAIL);
}

async fn enabled_only_visibility(store: &Arc<FakeStore>) {
    let key = key_caller(KEY_PRIVATE_GRANT);
    let listed = call(store, Some(&key), "/claude-code/plugins?enabled_only=true").await;
    assert_eq!(names(&listed), vec!["public-plugin"]);
    let unfiltered = call(store, Some(&key), "/claude-code/plugins?enabled_only=false").await;
    assert_eq!(names(&unfiltered), vec!["private-plugin", "public-plugin"]);
    assert_eq!(
        store.filters(),
        vec![ENABLED_FILTER, granted_filter(&["private-plugin"])]
    );
}

#[tokio::test]
async fn disabled_plugin_visibility() {
    admin_visibility(&visibility_store()).await;
    list_visibility(&visibility_store()).await;
    Box::pin(get_visibility(&visibility_store())).await;
    enabled_only_visibility(&visibility_store()).await;
}

#[tokio::test]
async fn list_where_clauses() {
    let store = visibility_store();
    let master_reply = call(&store, Some(&master()), "/claude-code/plugins").await;
    assert_eq!(master_reply.status, StatusCode::OK);
    let plain_reply = call(&store, Some(&plain_user()), "/claude-code/plugins").await;
    assert_eq!(plain_reply.status, StatusCode::OK);
    let grant_reply = call(
        &store,
        Some(&key_caller(KEY_PRIVATE_GRANT)),
        "/claude-code/plugins",
    )
    .await;
    assert_eq!(grant_reply.status, StatusCode::OK);
    let intersection_reply = call(
        &store,
        Some(&key_caller(KEY_INTERSECTION)),
        "/claude-code/plugins",
    )
    .await;
    assert_eq!(intersection_reply.status, StatusCode::OK);
    let disjoint_reply = call(
        &store,
        Some(&key_caller(KEY_DISJOINT)),
        "/claude-code/plugins",
    )
    .await;
    assert_eq!(disjoint_reply.status, StatusCode::OK);
    assert_eq!(
        store.filters(),
        vec![
            ALL_FILTER,
            ENABLED_FILTER,
            granted_filter(&["private-plugin"]),
            granted_filter(&["private-plugin"]),
            ENABLED_FILTER,
        ]
    );
}

#[tokio::test]
async fn granted_key_drops_rows_the_query_should_have_excluded() {
    let store = visibility_store();
    store.ignore_filters.store(true, Ordering::SeqCst);
    let reply = call(
        &store,
        Some(&key_caller(KEY_PRIVATE_GRANT)),
        "/claude-code/plugins",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(names(&reply), vec!["private-plugin", "public-plugin"]);
}

#[rstest]
#[case::list("/claude-code/plugins")]
#[case::item("/claude-code/plugins/route-plugin")]
#[tokio::test]
async fn route_aliases(#[case] path: &str) {
    let reply = booted_call(booted("oracle-master"), path, Some("Bearer oracle-master")).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.text,
        format!(r#"{{"detail":{{"error":"{DB_NOT_CONNECTED}"}}}}"#)
    );
}

#[rstest]
#[case::list("/claude-code/plugins")]
#[case::item("/claude-code/plugins/auth-plugin")]
#[tokio::test]
async fn auth_missing(#[case] path: &str) {
    let reply = booted_call(booted("oracle-master"), path, None).await;
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
        "/claude-code/plugins",
        Some("Bearer sk-not-the-master"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.text,
        r#"{"error":{"message":"the API key is not valid","type":"no_db_connection","param":null,"code":"400"}}"#
    );
}

#[tokio::test]
async fn invalid_enabled_only_is_rejected_before_the_database_check() {
    let reply = booted_call(
        booted("oracle-master"),
        "/claude-code/plugins?enabled_only=maybe",
        Some("Bearer oracle-master"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.text,
        r#"{"detail":[{"loc":["query","enabled_only"],"msg":"not a boolean","type":"bool_parsing"}]}"#
    );
}

#[tokio::test]
async fn visibility_is_checked_before_the_manifest_is_parsed() {
    let store = Arc::new(FakeStore::new());
    let mut broken = stored_row("private-broken", false, at(13, 0, 0));
    broken.manifest_json = Some("{not json".to_owned());
    store.seed(broken);
    let hidden = call(
        &store,
        Some(&plain_user()),
        "/claude-code/plugins/private-broken",
    )
    .await;
    assert_eq!(hidden.status, StatusCode::FORBIDDEN);
    assert_eq!(
        hidden.text,
        r#"{"detail":{"error":"Plugin 'private-broken' is not granted to this key"}}"#
    );
    let granted = call(
        &store,
        Some(&master()),
        "/claude-code/plugins/private-broken",
    )
    .await;
    assert_eq!(granted.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(granted.text, INVALID_MANIFEST_DETAIL);
    store.ignore_filters.store(true, Ordering::SeqCst);
    let listed = call(&store, Some(&plain_user()), "/claude-code/plugins").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.text, r#"{"plugins":[],"count":0}"#);
}

#[test]
fn decoder_reads_the_manifest_fields() {
    let fields = decode_claude_plugin_read(
        br#"{"source":{"repo":"org/repo"},"namespace":null,"extra":true}"#,
        &ApiLimits::default(),
    )
    .expect("the manifest decodes");
    assert_eq!(
        fields.source,
        Some(Json::Object(IndexMap::from([(
            "repo".to_owned(),
            Json::Text("org/repo".to_owned()),
        )]))),
    );
    assert_eq!(fields.namespace, Some(Json::Null));
    assert_eq!(fields.author, None);
    assert_eq!(fields.homepage, None);
}

#[test]
fn decoder_rejects_a_non_object_manifest() {
    let error = decode_claude_plugin_read(b"[]", &ApiLimits::default())
        .expect_err("a list is not a manifest");
    assert_eq!(
        error.to_string(),
        "the stored plugin manifest is not an object"
    );
}

#[test]
fn decoder_rejects_invalid_json() {
    let error = decode_claude_plugin_read(b"{not json", &ApiLimits::default())
        .expect_err("the bytes are not JSON");
    assert_eq!(
        error.to_string(),
        "the stored plugin manifest is not valid JSON"
    );
}

#[test]
fn decoder_rejects_bytes_over_the_limit() {
    let limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let error = decode_claude_plugin_read(b"{\"source\":{}}", &limits)
        .expect_err("the manifest is over the limit");
    assert_eq!(
        error.to_string(),
        "the stored plugin manifest is 13 bytes; the maximum is 4"
    );
}

fn core_row(name: &str, enabled: bool) -> PluginRow {
    plugin_row(
        stored_row(name, enabled, at(12, 0, 0)),
        &ApiLimits::default(),
    )
    .expect("the core row")
}

#[test]
fn run_filters_rows_visibility_refuses() {
    let list = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::List {
            enabled_only: false,
        },
        caller: None,
    };
    let result = list
        .run(vec![
            core_row("hidden-plugin", false),
            core_row("visible-plugin", true),
        ])
        .expect("the list");
    let listed = serde_json::to_value(&result).expect("the result serializes");
    assert_eq!(listed["count"].as_u64(), Some(1));
    assert_eq!(listed["plugins"][0]["name"], "visible-plugin");
    assert_eq!(listed["plugins"][0]["source"].to_string(), "{}");
    let get = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::Get {
            plugin_name: "hidden-plugin".to_owned(),
        },
        caller: None,
    };
    let denied = get
        .run(vec![core_row("hidden-plugin", false)])
        .expect_err("the row is not granted");
    assert_eq!(
        denied.to_string(),
        "Plugin 'hidden-plugin' is not granted to this key"
    );
    let missing = get.run(Vec::new()).expect_err("the row is missing");
    assert_eq!(missing.to_string(), "Plugin 'hidden-plugin' not found");
}

// deadpool builds its pool lazily, so a well-formed URL proves the store without a connection
#[tokio::test]
async fn database_store_resolves_the_configured_url() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let configured = loader
        .load(
            "general_settings:\n  master_key: oracle-master\n  database_url: postgresql://plugin:secret@127.0.0.1:5432/plugins\n",
        )
        .expect("the test config");
    assert!(super::database_store(&state_for(&configured)).is_some());
    let unconfigured = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: oracle-master\n")
    .expect("the test config");
    assert!(super::database_store(&state_for(&unconfigured)).is_none());
}

#[test]
fn replays_corpus() {
    let seeds = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/claude_plugin_read");
    let replayed = read_dir(seeds)
        .expect("the corpus directory exists")
        .map(|entry| {
            let seed = read(entry.expect("an entry is readable").path()).expect("a seed reads");
            let _decoded = decode_claude_plugin_read(&seed, &ApiLimits::default());
        })
        .count();
    assert!(replayed >= 8);
}
