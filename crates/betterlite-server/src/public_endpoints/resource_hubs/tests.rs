#![cfg(test)]

use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_core::api::claude_plugin_policy::DB_NOT_CONNECTED;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_policy_resources::ADD_CLAUDE_CODE_PLUGIN_TABLE;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::RwLock;
use tokio_postgres::NoTls;

use super::ResourceHubsState;
use super::agent_hub::public_agent_cards;
use super::error::ResourceHubsError;
use super::mcp_hub::public_mcp_servers;
use super::plugin_table::{PluginRow, PluginTable, PostgresPluginTable};
use super::plugin_types::ListPluginsResponse;
use super::skill_hub::{plugin_response, skill_hub};
use crate::agent_endpoints::admin_actions::ports::PublicGroupStore;
use crate::custom_url::CustomUrls;

struct Groups(Mutex<Option<Vec<String>>>);

impl Groups {
    fn new(groups: Option<Vec<String>>) -> Self {
        Self(Mutex::new(groups))
    }
}

impl PublicGroupStore for Groups {
    fn groups(&self) -> Option<Vec<String>> {
        self.0.lock().expect("the group store").clone()
    }

    fn set_groups(&self, groups: Vec<String>) {
        *self.0.lock().expect("the group store") = Some(groups);
    }
}

struct FakeTable {
    rows: Vec<PluginRow>,
    fail: bool,
}

#[async_trait]
impl PluginTable for FakeTable {
    async fn enabled_plugins(&self) -> Result<Vec<PluginRow>, TableError> {
        if self.fail {
            return Err(TableError::MissingRow);
        }
        Ok(self.rows.clone())
    }
}

fn agent(id: &str, card: &str) -> AgentResponse {
    let body = format!(r#"{{"agent_id":"{id}","agent_name":"{id}","agent_card_params":{card}}}"#);
    serde_json::from_str(&body).expect("the agent record")
}

fn registry_with(agents: Vec<AgentResponse>) -> AgentRegistry {
    let registry = AgentRegistry::default();
    for record in agents {
        registry.register_agent(record);
    }
    registry
}

fn custom_urls(proxy_base_url: Option<&str>) -> CustomUrls {
    CustomUrls {
        proxy_base_url: proxy_base_url.map(str::to_owned),
    }
}

fn full_card() -> &'static str {
    r#"{"protocolVersion":"0.3.0","name":"planner","description":"plans","version":"1.0.0","capabilities":{"streaming":true},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[],"iconUrl":null,"extra":"dropped"}"#
}

fn encoded(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("the value encodes")
}

#[test]
fn agent_hub_returns_no_agents_when_allowlist_is_none() {
    let registry = registry_with(vec![agent("alpha", full_card())]);
    let cards = public_agent_cards(&registry, None, &custom_urls(None), "http://test/", "")
        .expect("the cards build");
    assert!(cards.is_empty());
}

#[test]
fn agent_hub_filters_agents_by_allowlist_membership() {
    let registry = registry_with(vec![agent("alpha", full_card()), agent("beta", "{}")]);
    let groups = vec![String::from("beta")];
    let cards = public_agent_cards(
        &registry,
        Some(&groups),
        &custom_urls(None),
        "http://test/",
        "",
    )
    .expect("the cards build");
    assert_eq!(cards.len(), 1);
    let body = encoded(&cards[0]);
    assert!(body.contains(r#""name":"beta""#), "{body}");
    assert!(!body.contains("planner"), "{body}");
}

#[test]
fn agent_hub_excludes_agents_no_allowed_id_selects() {
    let registry = registry_with(vec![agent("alpha", "{}")]);
    let groups = vec![String::from("someone-else")];
    let cards = public_agent_cards(
        &registry,
        Some(&groups),
        &custom_urls(None),
        "http://test/",
        "",
    )
    .expect("the cards build");
    assert!(cards.is_empty());
}

#[test]
fn agent_hub_builds_public_url_from_request_base_and_agent_id() {
    let registry = registry_with(vec![agent("alpha", full_card())]);
    let groups = vec![String::from("alpha")];
    let cards = public_agent_cards(
        &registry,
        Some(&groups),
        &custom_urls(None),
        "http://testserver/",
        "",
    )
    .expect("the cards build");
    let body = encoded(&cards[0]);
    assert!(
        body.contains(r#""url":"http://testserver/a2a/alpha""#),
        "{body}"
    );
    assert!(body.contains(r#""iconUrl":null"#), "{body}");
    assert!(!body.contains("extra"), "{body}");
    assert!(body.contains(r#""name":"planner""#), "{body}");
}

#[test]
fn agent_hub_honours_proxy_base_url_and_root_path() {
    let registry = registry_with(vec![agent("alpha", "{}")]);
    let groups = vec![String::from("alpha")];
    let cards = public_agent_cards(
        &registry,
        Some(&groups),
        &custom_urls(Some("https://proxy.example")),
        "http://testserver/tenant/",
        "/tenant",
    )
    .expect("the cards build");
    let body = encoded(&cards[0]);
    assert!(
        body.contains(r#""url":"https://proxy.example/tenant/a2a/alpha""#),
        "{body}"
    );
}

#[test]
fn agent_hub_rejects_a_card_the_response_model_refuses() {
    let registry = registry_with(vec![agent("alpha", r#"{"skills":1}"#)]);
    let groups = vec![String::from("alpha")];
    let outcome = public_agent_cards(
        &registry,
        Some(&groups),
        &custom_urls(None),
        "http://test/",
        "",
    );
    let error = outcome.expect_err("the card is invalid");
    assert!(matches!(error, ResourceHubsError::AgentCard(_)));
}

fn settings(public_mcp_servers: &[&str], public_hub_strict: bool) -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: public_mcp_servers
            .iter()
            .map(|id| (*id).to_owned())
            .collect(),
        public_hub_strict,
        internal_ip_ranges: None,
    }
}

fn mcp_server(record: &str) -> MCPServer {
    serde_json::from_str(record).expect("the MCP server record")
}

fn listed_ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn mcp_hub_returns_no_servers_when_none_are_public() {
    let servers = vec![mcp_server(
        r#"{"server_id":"open","name":"Open","transport":"http","available_on_public_internet":true}"#,
    )];
    let registry = ServerRegistry::new(settings(&[], true), servers, Vec::new());
    let entries = public_mcp_servers(&registry, None).expect("the entries build");
    assert!(entries.is_empty());
}

#[test]
fn mcp_hub_lists_allowlisted_servers_and_marks_them_explicit() {
    let servers = vec![
        mcp_server(r#"{"server_id":"listed","name":"Listed","transport":"http"}"#),
        mcp_server(r#"{"server_id":"other","name":"Other","transport":"http"}"#),
    ];
    let registry = ServerRegistry::new(settings(&["listed"], true), servers, Vec::new());
    let explicit = listed_ids(&["listed"]);
    let entries = public_mcp_servers(&registry, Some(&explicit)).expect("the entries build");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].server_id, "listed");
    assert_eq!(
        entries[0].mcp_info.get("is_public"),
        Some(&JsonValue::Bool(true))
    );
    assert_eq!(
        entries[0].mcp_info.get("is_public_explicit"),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn mcp_hub_marks_unlisted_public_internet_servers_implicit() {
    let servers = vec![mcp_server(
        r#"{"server_id":"open","name":"Open","transport":"http","available_on_public_internet":true}"#,
    )];
    let registry = ServerRegistry::new(settings(&[], false), servers, Vec::new());
    let entries = public_mcp_servers(&registry, Some(&[])).expect("the entries build");
    assert_eq!(entries.len(), 1);
    let body = encoded(&entries[0]);
    assert!(body.contains(r#""is_public_explicit":false"#), "{body}");
    assert!(body.contains(r#""is_public":true"#), "{body}");
}

#[test]
fn mcp_hub_merges_stored_mcp_info_and_overrides_the_flags() {
    let servers = vec![mcp_server(
        r#"{"server_id":"listed","name":"Listed","alias":"Alias","server_name":"Remote","transport":"sse","spec_path":"/spec","auth_type":"oauth2","mcp_info":{"protocol_version":"2025-03-26","is_public":false}}"#,
    )];
    let registry = ServerRegistry::new(settings(&["listed"], true), servers, Vec::new());
    let explicit = listed_ids(&["listed"]);
    let entries = public_mcp_servers(&registry, Some(&explicit)).expect("the entries build");
    let body = encoded(&entries[0]);
    assert!(
        body.contains(r#""protocol_version":"2025-03-26""#),
        "{body}"
    );
    assert!(body.contains(r#""is_public":true"#), "{body}");
    assert!(body.contains(r#""is_public_explicit":true"#), "{body}");
    assert!(body.contains(r#""transport":"sse""#), "{body}");
    assert!(body.contains(r#""auth_type":"oauth2""#), "{body}");
}

#[test]
fn mcp_hub_serializes_nullable_fields_as_null() {
    let servers = vec![mcp_server(
        r#"{"server_id":"listed","name":"Listed","transport":"http"}"#,
    )];
    let registry = ServerRegistry::new(settings(&["listed"], true), servers, Vec::new());
    let explicit = listed_ids(&["listed"]);
    let entries = public_mcp_servers(&registry, Some(&explicit)).expect("the entries build");
    let body = encoded(&entries[0]);
    assert!(body.contains(r#""alias":null"#), "{body}");
    assert!(body.contains(r#""server_name":null"#), "{body}");
    assert!(body.contains(r#""spec_path":null"#), "{body}");
    assert!(body.contains(r#""auth_type":null"#), "{body}");
}

fn row(name: &str, manifest: Option<&str>) -> PluginRow {
    PluginRow {
        id: format!("plugin-{name}"),
        name: name.to_owned(),
        manifest_json: manifest.map(str::to_owned),
        enabled: true,
        created_at: None,
        updated_at: None,
    }
}

async fn response_of(
    rows: Vec<PluginRow>,
    fail: bool,
) -> Result<ListPluginsResponse, ResourceHubsError> {
    plugin_response(&FakeTable { rows, fail }).await
}

fn timestamp(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").expect("the timestamp parses")
}

#[tokio::test]
async fn skill_hub_parses_a_full_manifest() {
    let manifest = r#"{"source":{"source":"github","repo":"org/plugin"},"version":"2.1.0","description":"A plugin","author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://example.com","keywords":["one"],"category":"dev","domain":"Productivity","namespace":"workflows"}"#;
    let response = response_of(vec![row("full", Some(manifest))], false)
        .await
        .expect("the response builds");
    assert_eq!(response.count, 1);
    let item = &response.plugins[0];
    assert_eq!(item.version.as_deref(), Some("2.1.0"));
    assert_eq!(item.description.as_deref(), Some("A plugin"));
    assert_eq!(
        item.source.get("repo").map(String::as_str),
        Some("org/plugin")
    );
    assert_eq!(
        item.author.as_ref().map(|author| author.name.as_str()),
        Some("Ada")
    );
    assert_eq!(
        item.author
            .as_ref()
            .and_then(|author| author.email.as_deref()),
        Some("ada@example.com")
    );
    assert_eq!(item.keywords, Some(vec![String::from("one")]));
    assert_eq!(item.category.as_deref(), Some("dev"));
    assert_eq!(item.domain.as_deref(), Some("Productivity"));
    assert_eq!(item.namespace.as_deref(), Some("workflows"));
    assert_eq!(item.homepage.as_deref(), Some("https://example.com"));
    assert!(item.enabled);
}

#[tokio::test]
async fn skill_hub_defaults_every_missing_manifest_field() {
    let response = response_of(vec![row("bare", Some("{}"))], false)
        .await
        .expect("the response builds");
    let item = &response.plugins[0];
    assert!(item.source.is_empty());
    assert!(item.version.is_none());
    assert!(item.description.is_none());
    assert!(item.author.is_none());
    assert!(item.homepage.is_none());
    assert!(item.keywords.is_none());
    assert!(item.category.is_none());
    assert!(item.domain.is_none());
    assert!(item.namespace.is_none());
    assert_eq!(item.id, "plugin-bare");
    assert_eq!(item.name, "bare");
}

#[tokio::test]
async fn skill_hub_treats_an_absent_or_empty_manifest_as_empty() {
    for manifest in [None, Some("")] {
        let response = response_of(vec![row("empty", manifest)], false)
            .await
            .expect("the response builds");
        assert!(response.plugins[0].source.is_empty());
        assert!(response.plugins[0].version.is_none());
    }
}

#[tokio::test]
async fn skill_hub_rejects_a_manifest_that_is_not_json() {
    let error = response_of(vec![row("bad", Some("not json"))], false)
        .await
        .expect_err("the manifest is invalid");
    assert!(matches!(error, ResourceHubsError::Manifest(_)));
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn skill_hub_rejects_a_manifest_that_is_not_an_object() {
    for manifest in ["[]", "\"text\"", "null", "3"] {
        let outcome = response_of(vec![row("bad", Some(manifest))], false).await;
        assert!(
            matches!(outcome, Err(ResourceHubsError::Manifest(_))),
            "{manifest}"
        );
    }
}

#[tokio::test]
async fn skill_hub_rejects_a_null_source() {
    let error = response_of(vec![row("bad", Some(r#"{"source":null}"#))], false)
        .await
        .expect_err("a null source is refused");
    assert!(matches!(error, ResourceHubsError::ManifestSource));
}

#[tokio::test]
async fn skill_hub_rejects_a_source_that_is_not_a_string_map() {
    let error = response_of(vec![row("bad", Some(r#"{"source":{"repo":1}}"#))], false)
        .await
        .expect_err("the source values must be strings");
    assert!(matches!(error, ResourceHubsError::Manifest(_)));
}

#[tokio::test]
async fn skill_hub_rejects_a_wrongly_typed_string_field() {
    let error = response_of(vec![row("bad", Some(r#"{"version":2}"#))], false)
        .await
        .expect_err("the version must be a string");
    assert!(matches!(error, ResourceHubsError::Manifest(_)));
}

#[tokio::test]
async fn skill_hub_stringifies_timestamps_the_way_python_does() {
    let mut stored = row("timed", Some("{}"));
    stored.created_at = Some(timestamp("2025-01-02T03:04:05.123"));
    stored.updated_at = Some(timestamp("2025-01-02T03:04:05"));
    let response = response_of(vec![stored], false)
        .await
        .expect("the response builds");
    assert_eq!(
        response.plugins[0].created_at.as_deref(),
        Some("2025-01-02 03:04:05.123000+00:00")
    );
    assert_eq!(
        response.plugins[0].updated_at.as_deref(),
        Some("2025-01-02 03:04:05+00:00")
    );
}

#[tokio::test]
async fn skill_hub_leaves_absent_timestamps_null() {
    let response = response_of(vec![row("bare", Some("{}"))], false)
        .await
        .expect("the response builds");
    assert!(response.plugins[0].created_at.is_none());
    assert!(response.plugins[0].updated_at.is_none());
    let body = encoded(&response);
    assert!(body.contains(r#""created_at":null"#), "{body}");
    assert!(body.contains(r#""updated_at":null"#), "{body}");
}

#[tokio::test]
async fn skill_hub_reports_the_count_and_the_response_shape() {
    let response = response_of(vec![row("one", Some("{}")), row("two", Some("{}"))], false)
        .await
        .expect("the response builds");
    assert_eq!(response.count, 2);
    let body = encoded(&response);
    for key in [
        "\"plugins\"",
        "\"count\":2",
        "\"id\"",
        "\"name\"",
        "\"version\":null",
        "\"description\":null",
        "\"source\":{}",
        "\"author\":null",
        "\"homepage\":null",
        "\"keywords\":null",
        "\"category\":null",
        "\"domain\":null",
        "\"namespace\":null",
        "\"enabled\":true",
    ] {
        assert!(body.contains(key), "{key} missing from {body}");
    }
}

#[tokio::test]
async fn skill_hub_maps_a_table_error_to_500() {
    let error = response_of(Vec::new(), true)
        .await
        .expect_err("the table fails");
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn skill_hub_without_a_database_answers_500_with_the_policy_detail() {
    let state = ResourceHubsState {
        agents: Arc::new(AgentRegistry::default()),
        public_agent_groups: Arc::new(Groups::new(None)),
        mcp: Arc::new(RwLock::new(ServerRegistry::new(
            settings(&[], true),
            Vec::new(),
            Vec::new(),
        ))),
        public_mcp_servers: None,
        plugins: None,
        custom_urls: CustomUrls::default(),
    };
    let response = skill_hub(State(state))
        .await
        .expect_err("there is no database")
        .into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the body reads");
    let text = from_utf8(&body).expect("the body is UTF-8");
    assert_eq!(
        text,
        format!(r#"{{"detail":{{"error":"{DB_NOT_CONNECTED}"}}}}"#)
    );
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database(statements: Vec<String>) -> (PostgresFixture, Pool, PostgresPluginTable) {
    let url = Environment::parse_from(["resource-hubs-tests"])
        .url
        .map(SecretString::from);
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
    let store = PostgresPluginTable::new(Arc::new(PostgresStore {
        pool: Some(pool.clone()),
    }));
    (fixture, pool, store)
}

async fn plugin_database() -> (PostgresFixture, Pool, PostgresPluginTable) {
    let table = from_utf8(ADD_CLAUDE_CODE_PLUGIN_TABLE.bytes)
        .expect("the plugin table migration is UTF-8")
        .to_owned();
    database(vec![table]).await
}

async fn seed_plugin(pool: &Pool, name: &str, enabled: bool, created_at: NaiveDateTime) {
    let client = pool.get().await.expect("the pooled client");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ClaudeCodePluginTable" (id, name, version, description, manifest_json, files_json, enabled, created_at, updated_at) VALUES ($1, $2, '1.0.0', $3, $4, '{}', $5, $6, $6)"#,
            &[
                &format!("plugin-{name}"),
                &name,
                &format!("{name} description"),
                &format!(r#"{{"name":"{name}"}}"#),
                &enabled,
                &created_at,
            ],
        )
        .await
        .expect("the seeded plugin");
}

#[tokio::test]
async fn plugin_table_reads_only_enabled_rows() {
    let (_fixture, pool, store) = plugin_database().await;
    let now = timestamp("2025-01-02T03:04:05.123");
    seed_plugin(&pool, "enabled-plugin", true, now).await;
    seed_plugin(&pool, "disabled-plugin", false, now).await;
    let rows = store.enabled_plugins().await.expect("the plugin rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "enabled-plugin");
    assert_eq!(rows[0].id, "plugin-enabled-plugin");
    assert_eq!(
        rows[0].manifest_json.as_deref(),
        Some(r#"{"name":"enabled-plugin"}"#)
    );
    assert!(rows[0].enabled);
    assert_eq!(rows[0].created_at, Some(now));
    assert_eq!(rows[0].updated_at, Some(now));
}

#[tokio::test]
async fn plugin_table_surfaces_a_missing_table_as_an_error() {
    let (_fixture, _pool, store) = database(Vec::new()).await;
    let error = store
        .enabled_plugins()
        .await
        .expect_err("the plugin table does not exist");
    assert!(matches!(error, TableError::Database(_)));
}
