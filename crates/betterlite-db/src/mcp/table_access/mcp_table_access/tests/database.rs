#![cfg(test)]

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
use crate::mcp::table_access::rows::RowFields;
use crate::mcp::table_access::store::{_db_transaction_manager, McpTableStore};
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
    match row {
        JsonData::Object(fields) => match fields.get(name) {
            Some(JsonData::String(value)) => Some(value.as_str()),
            _ => None,
        },
        _ => None,
    }
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
        write: ServerWrite::Create(RowFields::texts([
            ("server_id", server_id),
            ("server_name", server_name),
            ("approval_status", "pending_review"),
        ])),
    }
}

#[tokio::test]
async fn a_locked_write_inserts_free_rows_and_reports_collisions() {
    let (_fixture, writer) = seeded().await;
    let mut transactions = _db_transaction_manager(&writer)
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
    let outcome = table
        .write_if_identifier_free(&free)
        .await
        .expect("a free identifier commits");
    let LockedWriteOutcome::Written(Some(row)) = outcome else {
        panic!("a free identifier writes the row");
    };
    assert_eq!(text_field(&row, "server_id"), Some("s2"));
    let taken = locked_write("s3", "taken");
    let outcome = table
        .write_if_identifier_free(&taken)
        .await
        .expect("the lock and check run");
    let LockedWriteOutcome::Conflict(conflict) = outcome else {
        panic!("a taken identifier is a conflict");
    };
    assert_eq!(conflict.field, IdentifierField::ServerName);
    assert_eq!(conflict.value, "taken");
    assert_eq!(conflict.server_id, "s1");
    let session = writer.session().await.expect("the fixture connects");
    let row = session
        .query_one(
            "SELECT COUNT(*) FROM \"LiteLLM_MCPServerTable\" WHERE \"server_id\" = 's3'",
            &[],
        )
        .await
        .expect("the table is readable");
    let inserted: i64 = row.get(0);
    assert_eq!(inserted, 0);
}
