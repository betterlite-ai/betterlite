#![cfg(test)]

use std::sync::Arc;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::store::McpTableStore;
use crate::postgres_store::session::PostgresStore;

const STORAGE: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id text PRIMARY KEY,
    server_name text,
    approval_status text,
    mcp_access_groups text[] NOT NULL DEFAULT '{}'
);
"#;

const SHARED_GROUP_ROWS: &str = r#"
INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, mcp_access_groups) VALUES
    ('shared-a', 'A', ARRAY['team-a', 'team-b']),
    ('shared-b', 'B', ARRAY['team-b']),
    ('disjoint', 'C', ARRAY['team-c']),
    ('no-groups', 'D', ARRAY[]::text[]);
"#;

const DRAFT_ROW: &str = r#"
INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, approval_status, mcp_access_groups) VALUES
    ('draft', 'Draft', 'draft', ARRAY['team-a']);
"#;

const WRITER_ROW: &str = r#"
INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, mcp_access_groups) VALUES
    ('writer-only', 'Writer', ARRAY['team-a']);
"#;

const READER_ROW: &str = r#"
INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, mcp_access_groups) VALUES
    ('reader-only', 'Reader', ARRAY['team-a']);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

async fn seeded(rows: &str) -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["mcp-access-group-servers-database"])
        .url
        .map(SecretString::from);
    let statements = format!("{STORAGE}\n{rows}");
    let fixture = PostgresFixture::provision_with_statements(configured.as_ref(), vec![statements])
        .resolve()
        .await
        .expect("the fixture provisions a database");
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

fn table<'db>(
    writer: &'db PostgresStore,
    reader: &'db PostgresStore,
    availability: &'db Availability,
) -> McpTableStore<'db> {
    let client = PrismaClient {
        writer,
        reader,
        availability,
    };
    McpTableStore::new(Some(client), publisher()).expect("a client builds a store")
}

fn groups(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test]
async fn rows_sharing_one_group_are_returned() {
    let (_fixture, store) = seeded(SHARED_GROUP_ROWS).await;
    let availability = Availability::default();
    let access = table(&store, &store, &availability);
    let mut found = access
        .server_ids_with_any_access_group(&groups(&["team-b"]), false)
        .await
        .expect("the overlap query runs");
    found.sort();
    assert_eq!(found, ["shared-a", "shared-b"]);
}

#[tokio::test]
async fn rows_with_disjoint_or_empty_arrays_are_not_returned() {
    let (_fixture, store) = seeded(SHARED_GROUP_ROWS).await;
    let availability = Availability::default();
    let access = table(&store, &store, &availability);
    let disjoint = access
        .server_ids_with_any_access_group(&groups(&["team-z"]), false)
        .await
        .expect("the overlap query runs");
    assert!(disjoint.is_empty());
    let empty = access
        .server_ids_with_any_access_group(&groups(&[]), false)
        .await
        .expect("an empty array overlaps nothing");
    assert!(empty.is_empty());
    let stored_empty = access
        .server_ids_with_any_access_group(&groups(&[""]), false)
        .await
        .expect("the overlap query runs");
    assert!(stored_empty.is_empty());
}

#[tokio::test]
async fn a_draft_row_still_counts() {
    let (_fixture, store) = seeded(DRAFT_ROW).await;
    let availability = Availability::default();
    let access = table(&store, &store, &availability);
    let found = access
        .server_ids_with_any_access_group(&groups(&["team-a"]), false)
        .await
        .expect("the overlap query runs");
    assert_eq!(found, ["draft"]);
}

#[tokio::test]
async fn with_use_writer_the_read_goes_to_the_writer() {
    let (_writer_fixture, writer) = seeded(WRITER_ROW).await;
    let (_reader_fixture, reader) = seeded(READER_ROW).await;
    let availability = Availability::default();
    let access = table(&writer, &reader, &availability);
    let requested = groups(&["team-a"]);
    assert_eq!(
        access
            .server_ids_with_any_access_group(&requested, false)
            .await
            .expect("the reader answers"),
        ["reader-only"]
    );
    assert_eq!(
        access
            .server_ids_with_any_access_group(&requested, true)
            .await
            .expect("the writer answers"),
        ["writer-only"]
    );
}
