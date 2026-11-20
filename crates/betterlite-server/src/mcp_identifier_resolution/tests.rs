#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_mcp::server_record::MCPServer;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::{
    ResolveError, ServerNames, db_servers_by_identifiers, identifier_matches,
    resolve_identifiers_to_ids,
};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT
);
"#;

const DROP_SERVERS_SQL: &str = r#"DROP TABLE "LiteLLM_MCPServerTable""#;

const INSERT_SERVER_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, alias) VALUES ($1, $2, $3)"#;

fn full_names() -> ServerNames<'static> {
    ServerNames {
        server_id: Some("srv-1"),
        alias: Some("alias-1"),
        server_name: Some("server-name-1"),
        name: Some("display-1"),
    }
}

#[rstest]
#[case::server_id("srv-1")]
#[case::alias("alias-1")]
#[case::server_name("server-name-1")]
#[case::name("display-1")]
fn matches_each_present_identity_attribute(#[case] identifier: &str) {
    assert!(identifier_matches(&full_names(), identifier));
}

#[rstest]
#[case::unknown_value("other")]
#[case::prefix_of_server_id("srv")]
#[case::suffix_of_alias("alias-1x")]
#[case::empty("")]
fn matches_nothing_when_no_attribute_holds_the_identifier(#[case] identifier: &str) {
    assert!(!identifier_matches(&full_names(), identifier));
}

#[test]
fn matches_never_uses_a_missing_name() {
    let stored = ServerNames {
        server_id: Some("srv-1"),
        alias: Some("alias-1"),
        server_name: None,
        name: None,
    };
    assert!(!identifier_matches(&stored, "display-1"));
    let absent = ServerNames {
        server_id: None,
        alias: None,
        server_name: None,
        name: None,
    };
    assert!(!identifier_matches(&absent, ""));
}

#[test]
fn matches_treats_an_empty_attribute_as_present() {
    let names = ServerNames {
        server_id: None,
        alias: Some(""),
        server_name: None,
        name: None,
    };
    assert!(identifier_matches(&names, ""));
}

fn strings(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

fn registry_record(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct TestDatabase {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    availability: Availability,
}

impl TestDatabase {
    async fn provision() -> Self {
        let url = Environment::parse_from(["mcp-identifier-resolution-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .unwrap();
        let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
        let pool = Pool::builder(manager).max_size(4).build().unwrap();
        Self {
            fixture,
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            pool,
            availability: Availability::default(),
        }
    }

    async fn insert(&self, server_id: &str, server_name: Option<&str>, alias: Option<&str>) {
        let client = self.pool.get().await.unwrap();
        client
            .execute(INSERT_SERVER_SQL, &[&server_id, &server_name, &alias])
            .await
            .unwrap();
    }

    async fn drop_servers_table(&self) {
        let client = self.pool.get().await.unwrap();
        client.batch_execute(DROP_SERVERS_SQL).await.unwrap();
    }

    fn prisma(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

fn offline_prisma<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
) -> PrismaClient<'db> {
    PrismaClient {
        writer: store,
        reader: store,
        availability,
    }
}

#[tokio::test]
async fn resolve_unites_database_and_registry_matches() {
    let database = TestDatabase::provision().await;
    database
        .insert("db-1", Some("db-server-name"), Some("shared-alias"))
        .await;
    let config = registry_record(
        r#"{"server_id": "cfg-1", "name": "cfg-display", "alias": "shared-alias", "server_name": "cfg-server-name", "transport": "http"}"#,
    );
    let identifiers = strings(&[
        "db-1",
        "shared-alias",
        "cfg-display",
        "cfg-server-name",
        "db-server-name",
        "nobody",
    ]);
    let resolved = resolve_identifiers_to_ids(&identifiers, Some(database.prisma()), &[&config])
        .await
        .unwrap();
    assert_eq!(resolved["db-1"], strings(&["db-1"]));
    assert_eq!(resolved["shared-alias"], strings(&["cfg-1", "db-1"]));
    assert_eq!(resolved["cfg-display"], strings(&["cfg-1"]));
    assert_eq!(resolved["cfg-server-name"], strings(&["cfg-1"]));
    assert_eq!(resolved["db-server-name"], strings(&["db-1"]));
    assert!(resolved["nobody"].is_empty());
    database.close().await;
}

#[tokio::test]
async fn resolve_joins_the_registry_without_a_client() {
    let config = registry_record(
        r#"{"server_id": "cfg-1", "name": "cfg-display", "alias": "cfg-alias", "transport": "http"}"#,
    );
    let identifiers = strings(&["cfg-alias", "db-1"]);
    let resolved = resolve_identifiers_to_ids(&identifiers, None, &[&config])
        .await
        .unwrap();
    assert_eq!(resolved["cfg-alias"], strings(&["cfg-1"]));
    assert!(resolved["db-1"].is_empty());
}

#[tokio::test]
async fn resolve_skips_rows_and_records_with_an_empty_server_id() {
    let database = TestDatabase::provision().await;
    database
        .insert("", Some("empty-row-name"), Some("row-alias"))
        .await;
    let config = registry_record(
        r#"{"server_id": "", "name": "empty-record-name", "alias": "record-alias", "transport": "http"}"#,
    );
    let identifiers = strings(&["row-alias", "record-alias", "empty-record-name", ""]);
    let resolved = resolve_identifiers_to_ids(&identifiers, Some(database.prisma()), &[&config])
        .await
        .unwrap();
    assert!(resolved["row-alias"].is_empty());
    assert!(resolved["record-alias"].is_empty());
    assert!(resolved["empty-record-name"].is_empty());
    assert!(resolved[""].is_empty());
    database.close().await;
}

#[tokio::test]
async fn resolve_returns_an_empty_map_without_identifiers() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let config =
        registry_record(r#"{"server_id": "cfg-1", "name": "cfg-display", "transport": "http"}"#);
    let prisma = offline_prisma(&store, &availability);
    let resolved = resolve_identifiers_to_ids(&BTreeSet::new(), Some(prisma), &[&config])
        .await
        .unwrap();
    assert!(resolved.is_empty());
}

#[tokio::test]
async fn resolve_lookup_returns_no_rows_without_a_client_or_identifiers() {
    let identifiers = strings(&["db-1"]);
    assert!(
        db_servers_by_identifiers(&identifiers, None)
            .await
            .unwrap()
            .is_empty()
    );
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let prisma = offline_prisma(&store, &availability);
    assert!(
        db_servers_by_identifiers(&BTreeSet::new(), Some(prisma))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn resolve_lookup_finds_rows_by_server_id_alias_and_server_name() {
    let database = TestDatabase::provision().await;
    database.insert("by-id", Some("row-name-a"), None).await;
    database
        .insert("by-alias-row", Some("row-name-b"), Some("alias-target"))
        .await;
    database
        .insert("by-server-name-row", Some("server-name-target"), None)
        .await;
    database
        .insert("unrelated", Some("unrelated-name"), Some("unrelated-alias"))
        .await;
    let identifiers = strings(&["by-id", "alias-target", "server-name-target"]);
    let mut found = db_servers_by_identifiers(&identifiers, Some(database.prisma()))
        .await
        .unwrap();
    found.sort_by(|left, right| left.server_id.cmp(&right.server_id));
    let found_ids: Vec<&str> = found.iter().map(|row| row.server_id.as_str()).collect();
    assert_eq!(found_ids, ["by-alias-row", "by-id", "by-server-name-row"]);
    assert_eq!(found[0].alias.as_deref(), Some("alias-target"));
    assert!(found[1].alias.is_none());
    assert_eq!(found[1].server_name.as_deref(), Some("row-name-a"));
    assert_eq!(found[2].server_name.as_deref(), Some("server-name-target"));
    database.close().await;
}

#[tokio::test]
async fn resolve_returns_a_database_error_before_reading_the_registry() {
    let database = TestDatabase::provision().await;
    database.drop_servers_table().await;
    let config =
        registry_record(r#"{"server_id": "cfg-1", "name": "cfg-display", "transport": "http"}"#);
    let identifiers = strings(&["cfg-display"]);
    let outcome =
        resolve_identifiers_to_ids(&identifiers, Some(database.prisma()), &[&config]).await;
    assert!(matches!(outcome, Err(ResolveError::Database(_))));
    database.close().await;
}
