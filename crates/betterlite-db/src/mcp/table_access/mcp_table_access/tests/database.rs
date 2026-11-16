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
use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::lock::mcp_server_write_if_identifier_free;
use crate::mcp::table_access::rows::{KeyValues, RowFields, ServerUpdate};
use crate::mcp::table_access::store::{LockedTable, McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::{
    IdentifierField, LockedWrite, LockedWriteOutcome, McpServerAccess as _, ServerWrite,
};
use crate::mcp::table_access::user_tables::{
    db_find_user_credential_row, db_find_user_credential_rows, db_find_user_env_var_rows,
    db_upsert_user_credential_row,
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
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    id text PRIMARY KEY,
    user_id text NOT NULL,
    server_id text NOT NULL,
    credential_b64 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    id text PRIMARY KEY,
    user_id text NOT NULL,
    server_id text NOT NULL,
    values_b64 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, server_id)
);
INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64)
VALUES ('e1', 'u1', 's1', 'blob-1'), ('e2', 'u2', 's1', 'blob-2');
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

#[tokio::test]
async fn the_transaction_table_reads_and_updates_inside_the_open_transaction() {
    let (_fixture, writer) = seeded().await;
    let mut transactions = db_transaction_manager(&writer)
        .await
        .expect("the manager opens on a live pool");
    let transaction = transactions.begin().await.expect("the transaction opens");
    let locked = LockedTable {
        transaction: &transaction,
    };
    let update = ServerUpdate {
        server_id: "s1".to_owned(),
        data: RowFields::texts(&[("approval_status", "active")]),
    };
    let updated = locked
        .update_server(&update)
        .await
        .expect("the update runs")
        .expect("s1 is stored");
    assert_eq!(text_field(&updated, "approval_status"), Some("active"));
    let active = FindServers {
        filter: Some(ServerFilter::ApprovalStatus("active".to_owned())),
        descending: false,
        take: None,
    };
    let seen = locked
        .find_servers(&active)
        .await
        .expect("the read runs inside the transaction");
    assert_eq!(seen, vec![updated]);
    transaction.rollback().await.expect("the transaction ends");
    assert_eq!(stored_count(&writer, "s1").await, 1);
}

#[tokio::test]
async fn a_credential_upsert_keeps_the_row_and_rewrites_only_the_blob() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let table = McpTableStore::new(Some(client), publisher()).expect("a client builds a store");
    db_upsert_user_credential_row(&table, "u1", "s1", "first")
        .await
        .expect("the create arm inserts");
    let created = db_find_user_credential_row(&table, "u1", "s1")
        .await
        .expect("the credential is readable")
        .expect("u1 has a credential for s1");
    db_upsert_user_credential_row(&table, "u1", "s1", "second")
        .await
        .expect("the update arm rewrites");
    let updated = db_find_user_credential_row(&table, "u1", "s1")
        .await
        .expect("the credential is readable")
        .expect("the credential is still stored");
    assert_eq!(text_field(&updated, "credential_b64"), Some("second"));
    assert_eq!(text_field(&updated, "id"), text_field(&created, "id"));
    let missing = db_find_user_credential_row(&table, "u1", "s2")
        .await
        .expect("the read runs");
    assert_eq!(missing, None);
    let every = db_find_user_credential_rows(&table, None)
        .await
        .expect("the read runs");
    assert_eq!(every.len(), 1);
}

#[tokio::test]
async fn env_var_rows_are_read_with_the_given_where_or_unfiltered() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let table = McpTableStore::new(Some(client), publisher()).expect("a client builds a store");
    let key = KeyValues::from([("user_id", "u2".to_owned())]);
    let filtered = db_find_user_env_var_rows(&table, Some(&key))
        .await
        .expect("the read runs");
    let blobs: Vec<Option<&str>> = filtered
        .iter()
        .map(|stored| text_field(stored, "values_b64"))
        .collect();
    assert_eq!(blobs, vec![Some("blob-2")]);
    let every = db_find_user_env_var_rows(&table, None)
        .await
        .expect("the read runs");
    assert_eq!(every.len(), 2);
}
