#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::ready;
use std::sync::Mutex;

use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_mcp::server_record::MCPServer;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::{
    DisplayNameError, DisplayNameStore, agent_display_names, key_display_names,
    mcp_server_display_names,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    McpServers,
    Agents,
    Keys,
}

#[derive(Clone)]
enum Reply {
    Names(BTreeMap<String, String>),
    Failed,
}

impl Default for Reply {
    fn default() -> Self {
        Self::Names(BTreeMap::new())
    }
}

#[derive(Default)]
struct FakeStore {
    mcp_servers: Reply,
    agents: Reply,
    keys: Reply,
    calls: Mutex<Vec<(Family, Vec<String>)>>,
}

impl FakeStore {
    fn answering(family: Family, names: BTreeMap<String, String>) -> Self {
        let mut store = Self::default();
        match family {
            Family::McpServers => store.mcp_servers = Reply::Names(names),
            Family::Agents => store.agents = Reply::Names(names),
            Family::Keys => store.keys = Reply::Names(names),
        }
        store
    }

    fn failing() -> Self {
        Self {
            mcp_servers: Reply::Failed,
            agents: Reply::Failed,
            keys: Reply::Failed,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, family: Family, ids: &BTreeSet<String>) {
        let requested = ids.iter().cloned().collect();
        self.calls.lock().unwrap().push((family, requested));
    }

    fn calls(&self) -> Vec<(Family, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    fn reply(reply: &Reply) -> Result<BTreeMap<String, String>, DisplayNameError> {
        match reply {
            Reply::Names(names) => Ok(names.clone()),
            Reply::Failed => Err(DisplayNameError::Table(TableError::MissingRow)),
        }
    }
}

impl DisplayNameStore for FakeStore {
    fn mcp_server_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        self.record(Family::McpServers, &ids);
        ready(Self::reply(&self.mcp_servers))
    }

    fn agent_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        self.record(Family::Agents, &ids);
        ready(Self::reply(&self.agents))
    }

    fn key_names(
        &self,
        tokens: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        self.record(Family::Keys, &tokens);
        ready(Self::reply(&self.keys))
    }
}

async fn lookup<S>(
    family: Family,
    store: &S,
    ids: &[String],
    config: &BTreeMap<String, MCPServer>,
    registry: &AgentRegistry,
) -> Result<BTreeMap<String, String>, DisplayNameError>
where
    S: DisplayNameStore + ?Sized,
{
    match family {
        Family::McpServers => mcp_server_display_names(store, ids, config).await,
        Family::Agents => agent_display_names(store, ids, registry).await,
        Family::Keys => key_display_names(store, ids).await,
    }
}

fn names_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), String::from(*value)))
        .collect()
}

fn quoted(value: Option<&str>) -> String {
    value.map_or_else(|| String::from("null"), |text| format!("\"{text}\""))
}

fn config_server(
    server_id: &str,
    name: &str,
    alias: Option<&str>,
    server_name: Option<&str>,
) -> MCPServer {
    let json = format!(
        r#"{{"server_id":"{server_id}","name":"{name}","transport":"http","alias":{},"server_name":{}}}"#,
        quoted(alias),
        quoted(server_name)
    );
    serde_json::from_str(&json).unwrap()
}

fn config_registry(names: &[&str]) -> AgentRegistry {
    let entries: Vec<Json> = names
        .iter()
        .map(|name| {
            serde_json::from_str(&format!(
                r#"{{"agent_name":"{name}","agent_card_params":{{"model":"m"}}}}"#
            ))
            .unwrap()
        })
        .collect();
    let registry = AgentRegistry::default();
    registry.load_agents_from_config(Some(&entries)).unwrap();
    registry
}

#[rstest]
#[case::mcp_servers(Family::McpServers)]
#[case::agents(Family::Agents)]
#[case::keys(Family::Keys)]
#[tokio::test]
async fn empty_ids_skip_the_store(#[case] family: Family) {
    let store = FakeStore::default();
    let names = lookup(
        family,
        &store,
        &[],
        &BTreeMap::new(),
        &AgentRegistry::default(),
    )
    .await
    .unwrap();
    assert!(names.is_empty());
    assert!(store.calls().is_empty());
}

#[rstest]
#[case::mcp_servers(Family::McpServers)]
#[case::agents(Family::Agents)]
#[case::keys(Family::Keys)]
#[tokio::test]
async fn repeated_ids_are_deduplicated_before_the_read(#[case] family: Family) {
    let store = FakeStore::answering(family, names_map(&[("x", "label")]));
    let ids = vec![
        String::from("x"),
        String::from("x"),
        String::from("missing"),
    ];
    let names = lookup(
        family,
        &store,
        &ids,
        &BTreeMap::new(),
        &AgentRegistry::default(),
    )
    .await
    .unwrap();
    assert_eq!(names, names_map(&[("x", "label")]));
    assert_eq!(
        store.calls(),
        vec![(family, vec![String::from("missing"), String::from("x")])]
    );
}

#[rstest]
#[case::mcp_servers(Family::McpServers)]
#[case::agents(Family::Agents)]
#[case::keys(Family::Keys)]
#[tokio::test]
async fn a_store_failure_propagates_without_a_second_read(#[case] family: Family) {
    let store = FakeStore::failing();
    let error = lookup(
        family,
        &store,
        &[String::from("x")],
        &BTreeMap::new(),
        &AgentRegistry::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        DisplayNameError::Table(TableError::MissingRow)
    ));
    assert_eq!(store.calls().len(), 1);
}

#[rstest]
#[case::alias_comes_first(Some("Alias"), Some("server_name"), "Alias")]
#[case::server_name_second(None, Some("server_name"), "server_name")]
#[case::registry_name_last(None, None, "config-name")]
#[case::empty_alias_is_used_as_given(Some(""), Some("server_name"), "")]
#[tokio::test]
async fn mcp_config_servers_fall_back_alias_then_server_name_then_name(
    #[case] alias: Option<&str>,
    #[case] server_name: Option<&str>,
    #[case] expected: &str,
) {
    let store = FakeStore::default();
    let config = BTreeMap::from([
        (
            String::from("s1"),
            config_server("s1", "config-name", alias, server_name),
        ),
        (
            String::from("other"),
            config_server("other", "not-requested", None, None),
        ),
    ]);
    let names = mcp_server_display_names(&store, &[String::from("s1")], &config)
        .await
        .unwrap();
    assert_eq!(names, names_map(&[("s1", expected)]));
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT
);
CREATE TABLE "LiteLLM_AgentsTable" (
    agent_id TEXT PRIMARY KEY,
    agent_name TEXT NOT NULL
);
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT
);
"#;

const INSERT_MCP_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, alias, server_name) VALUES ($1, $2, $3)"#;
const INSERT_AGENT_SQL: &str =
    r#"INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name) VALUES ($1, $2)"#;
const INSERT_TOKEN_SQL: &str =
    r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias) VALUES ($1, $2)"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["management-display-names-tests"])
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

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn execute(pool: &Pool, sql: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.unwrap();
    client.execute(sql, parameters).await.unwrap();
}

struct DatabaseClientFixture {
    store: PostgresStore,
    availability: Availability,
}

impl DatabaseClientFixture {
    fn new(pool: &Pool) -> Self {
        Self {
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            availability: Availability::default(),
        }
    }

    fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }
}

async fn seed_mcp_rows(pool: &Pool) {
    execute(
        pool,
        INSERT_MCP_SQL,
        &[&"s1", &Some("db-alias"), &None::<&str>],
    )
    .await;
    execute(pool, INSERT_MCP_SQL, &[&"s2", &None::<&str>, &None::<&str>]).await;
    execute(
        pool,
        INSERT_MCP_SQL,
        &[&"s3", &Some(""), &Some("kept-server-name")],
    )
    .await;
    execute(pool, INSERT_MCP_SQL, &[&"s4", &None::<&str>, &None::<&str>]).await;
}

fn mcp_config() -> BTreeMap<String, MCPServer> {
    BTreeMap::from([
        (
            String::from("s1"),
            config_server("s1", "config-name", Some("config-alias"), None),
        ),
        (
            String::from("s2"),
            config_server("s2", "config-name", None, None),
        ),
        (
            String::from("s3"),
            config_server("s3", "config-name", Some("config-alias"), None),
        ),
        (
            String::from("unrequested"),
            config_server("unrequested", "other", None, None),
        ),
    ])
}

#[tokio::test]
async fn mcp_database_names_win_and_nameless_rows_keep_the_configured_label() {
    let (fixture, pool) = database().await;
    seed_mcp_rows(&pool).await;
    let fixture_client = DatabaseClientFixture::new(&pool);
    let client = fixture_client.client();
    let ids = [
        String::from("s1"),
        String::from("s1"),
        String::from("s2"),
        String::from("s3"),
        String::from("s4"),
        String::from("missing"),
    ];
    let names = mcp_server_display_names(&client, &ids, &mcp_config())
        .await
        .unwrap();
    assert_eq!(
        names,
        names_map(&[
            ("s1", "db-alias"),
            ("s2", "config-name"),
            ("s3", "kept-server-name"),
        ])
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn agent_stored_names_win_and_blank_rows_keep_their_empty_label() {
    let (fixture, pool) = database().await;
    let registry = config_registry(&["alpha", "beta", "gamma"]);
    let agents = registry.get_agent_list(None);
    let alpha = agents.first().unwrap();
    let beta = &agents[1];
    let stable_alpha = alpha.agent_id.clone();
    let stable_beta = beta.agent_id.clone();
    let legacy_alpha = registry
        .ids_for_agent(&stable_alpha)
        .into_iter()
        .find(|id| id != &stable_alpha)
        .unwrap();
    execute(&pool, INSERT_AGENT_SQL, &[&stable_alpha, &"stored-name"]).await;
    execute(&pool, INSERT_AGENT_SQL, &[&stable_beta, &""]).await;
    let fixture_client = DatabaseClientFixture::new(&pool);
    let client = fixture_client.client();
    let ids = vec![
        stable_alpha.clone(),
        legacy_alpha.clone(),
        stable_beta.clone(),
        String::from("unknown"),
    ];
    let names = agent_display_names(&client, &ids, &registry).await.unwrap();
    assert_eq!(
        names,
        names_map(&[
            (stable_alpha.as_str(), "stored-name"),
            (legacy_alpha.as_str(), "alpha"),
            (stable_beta.as_str(), ""),
        ])
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn key_labels_come_only_from_stored_aliases() {
    let (fixture, pool) = database().await;
    execute(&pool, INSERT_TOKEN_SQL, &[&"k1", &Some("ci-key")]).await;
    execute(&pool, INSERT_TOKEN_SQL, &[&"k2", &None::<&str>]).await;
    execute(&pool, INSERT_TOKEN_SQL, &[&"k3", &Some("")]).await;
    execute(&pool, INSERT_TOKEN_SQL, &[&"k4", &Some("other-key")]).await;
    let fixture_client = DatabaseClientFixture::new(&pool);
    let client = fixture_client.client();
    let ids = vec![
        String::from("k1"),
        String::from("k2"),
        String::from("k1"),
        String::from("k3"),
        String::from("missing"),
    ];
    let names = key_display_names(&client, &ids).await.unwrap();
    assert_eq!(names, names_map(&[("k1", "ci-key")]));
    close(fixture, pool).await;
}

#[tokio::test]
async fn mcp_names_without_a_database_are_an_error() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let error = mcp_server_display_names(&client, &[String::from("s1")], &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, DisplayNameError::Database(_)));
}

#[tokio::test]
async fn agent_names_without_a_database_are_an_error() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let error = agent_display_names(&client, &[String::from("a1")], &AgentRegistry::default())
        .await
        .unwrap_err();
    assert!(matches!(error, DisplayNameError::Database(_)));
}

#[tokio::test]
async fn key_names_without_a_database_are_an_error() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let error = key_display_names(&client, &[String::from("k1")])
        .await
        .unwrap_err();
    assert!(matches!(error, DisplayNameError::Token(_)));
}

#[tokio::test]
async fn empty_ids_reach_no_database_at_all() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    assert!(
        mcp_server_display_names(&client, &[], &BTreeMap::new())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        agent_display_names(&client, &[], &AgentRegistry::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(key_display_names(&client, &[]).await.unwrap().is_empty());
}
