use std::sync::Arc;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::{GenericClient as _, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::mcp_server_write_if_identifier_free;
use crate::mcp::table_access::rows::{RowFields, ServerUpdate};
use crate::mcp::table_access::store::{McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::{
    IdentifierField, LockedWrite, LockedWriteOutcome, ServerWrite,
};
use crate::postgres_store::session::PostgresStore;

const STORAGE: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id text PRIMARY KEY,
    server_name text,
    alias text,
    approval_status text,
    submitted_by text,
    submitted_at timestamptz,
    env jsonb,
    pinned_tools jsonb,
    updated_at timestamptz
);
INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, approval_status)
VALUES ('s1', 'TAKEN', 'pending_review');
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

fn text_field<'row>(row: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = row else {
        return None;
    };
    let Some(JsonData::String(value)) = fields.get(name) else {
        return None;
    };
    Some(value)
}

async fn seeded() -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["mcp-table-access-database"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .expect("the fixture provisions a schema");
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap();
    pool.get()
        .await
        .unwrap()
        .batch_execute(STORAGE)
        .await
        .expect("the test table exists");
    let store = PostgresStore { pool: Some(pool) };
    (fixture, store)
}

fn locked_write(server_id: &'static str, server_name: &str) -> LockedWrite {
    LockedWrite {
        server_name: Some(server_name.to_owned()),
        alias: None,
        exclude_server_id: None,
        write: ServerWrite::Create(RowFields::texts(&[
            ("server_id", server_id),
            ("server_name", server_name),
            ("approval_status", "pending_review"),
        ])),
    }
}

async fn stored_count(writer: &PostgresStore, server_id: &str) -> i64 {
    let session = writer.session().await.expect("the fixture connects");
    let counted = session
        .query_one(
            "SELECT COUNT(*) FROM \"LiteLLM_MCPServerTable\" WHERE \"server_id\" = $1",
            &[&server_id],
        )
        .await
        .expect("the table is readable");
    counted.get(0)
}

#[tokio::test]
async fn a_locked_write_inserts_free_rows_and_reports_collisions() {
    let (_fixture, writer) = seeded().await;
    let mut transactions = db_transaction_manager(&writer)
        .await
        .expect("the manager opens on a live pool");
    let transaction = transactions.begin().await.expect("the transaction opens");
    transaction
        .rollback()
        .await
        .expect("the empty transaction ends");
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let table = McpTableStore::new(Some(client), publisher()).expect("a client builds a store");
    let free = locked_write("s2", "OTHER");
    let written = mcp_server_write_if_identifier_free(&table, &free)
        .await
        .expect("a free identifier commits");
    let LockedWriteOutcome::Written(Some(inserted)) = written else {
        panic!("a free identifier writes the row");
    };
    assert_eq!(text_field(&inserted, "server_id"), Some("s2"));
    assert_eq!(stored_count(&writer, "s2").await, 1);
    let taken = locked_write("s3", "taken");
    let refused = mcp_server_write_if_identifier_free(&table, &taken)
        .await
        .expect("the lock and check run");
    let LockedWriteOutcome::Conflict(conflict) = refused else {
        panic!("a taken identifier is a conflict");
    };
    assert_eq!(conflict.field, IdentifierField::ServerName);
    assert_eq!(conflict.value, "taken");
    assert_eq!(conflict.server_id, "s1");
    assert_eq!(stored_count(&writer, "s3").await, 0);
}

#[tokio::test]
async fn a_locked_update_may_keep_its_own_name_and_changes_the_row() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let table = McpTableStore::new(Some(client), publisher()).expect("a client builds a store");
    let rename = LockedWrite {
        server_name: Some("taken".to_owned()),
        alias: None,
        exclude_server_id: Some("s1".to_owned()),
        write: ServerWrite::Update(ServerUpdate {
            server_id: "s1".to_owned(),
            data: RowFields::texts(&[("approval_status", "active")]),
        }),
    };
    let written = mcp_server_write_if_identifier_free(&table, &rename)
        .await
        .expect("the own row is excluded from the check");
    let LockedWriteOutcome::Written(Some(updated)) = written else {
        panic!("the update returns the row");
    };
    assert_eq!(text_field(&updated, "approval_status"), Some("active"));
    let missing = LockedWrite {
        server_name: None,
        alias: Some("fresh".to_owned()),
        exclude_server_id: Some("absent".to_owned()),
        write: ServerWrite::Update(ServerUpdate {
            server_id: "absent".to_owned(),
            data: RowFields::texts(&[("approval_status", "active")]),
        }),
    };
    let nothing = mcp_server_write_if_identifier_free(&table, &missing)
        .await
        .expect("an update of no row still commits");
    assert!(matches!(nothing, LockedWriteOutcome::Written(None)));
}
