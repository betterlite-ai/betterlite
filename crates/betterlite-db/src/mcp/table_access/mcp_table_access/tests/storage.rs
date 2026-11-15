use std::sync::Arc;

use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::actions::mcp_server_table_actions;
use crate::mcp::table_access::filter::FindServers;
use crate::mcp::table_access::mcp_server_write_if_identifier_free;
use crate::mcp::table_access::rows::{CREDENTIAL_TABLE, KeyValues, RowFields, ServerUpdate};
use crate::mcp::table_access::sql;
use crate::mcp::table_access::store::{McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::{LockedWrite, McpServerAccess, McpTableError, ServerWrite};
use crate::mcp::table_access::user_tables::{
    db_find_user_credential_row, db_find_user_credential_rows, db_find_user_env_var_rows,
    db_upsert_user_credential_row,
};
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

fn store<'db>(writer: &'db PostgresStore, availability: &'db Availability) -> McpTableStore<'db> {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    McpTableStore::new(Some(client), publisher()).expect("a client builds a store")
}

fn row() -> RowFields {
    RowFields::texts(&[("server_id", "s1")])
}

fn no_database(error: &TableError) -> bool {
    matches!(error, TableError::Database(DbError::NoDbConnected))
}

#[tokio::test]
async fn sql_reads_without_a_pool_report_no_database() {
    let pool_less = PostgresStore { pool: None };
    let keyed = sql::select_keyed(&pool_less, &CREDENTIAL_TABLE, None)
        .await
        .expect_err("no pool means no rows");
    assert!(matches!(
        keyed,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let statement = Ok(("SELECT 1".to_owned(), Vec::new()));
    let ran = sql::run(&pool_less, statement)
        .await
        .expect_err("no pool means no session");
    assert!(no_database(&ran));
    let refused = sql::run(&pool_less, Err(TableError::MissingRow))
        .await
        .expect_err("a statement that failed to build never runs");
    assert!(matches!(refused, TableError::MissingRow));
}

#[tokio::test]
async fn table_actions_without_a_pool_report_no_database() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let table =
        mcp_server_table_actions(Some(client), publisher()).expect("a client builds table actions");
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let created = table
        .create(row())
        .await
        .expect_err("no pool means no insert");
    assert!(no_database(&created));
    let created_many = table
        .create_many(vec![row()])
        .await
        .expect_err("no pool means no insert");
    assert!(no_database(&created_many));
    let updated = table
        .update(row(), filter.clone())
        .await
        .expect_err("no pool means no update");
    assert!(no_database(&updated));
    let updated_many = table
        .update_many(row(), filter.clone())
        .await
        .expect_err("no pool means no update");
    assert!(no_database(&updated_many));
    let upserted = table
        .upsert(KeyValues::new(), row())
        .await
        .expect_err("no pool means no upsert");
    assert!(no_database(&upserted));
    let deleted = table
        .delete(filter.clone())
        .await
        .expect_err("no pool means no delete");
    assert!(no_database(&deleted));
    let deleted_many = table
        .delete_many(filter)
        .await
        .expect_err("no pool means no delete");
    assert!(no_database(&deleted_many));
}

#[tokio::test]
async fn a_pool_less_store_reports_no_database_for_reads_updates_and_locked_writes() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let opened = db_transaction_manager(&writer)
        .await
        .err()
        .expect("no pool means no session");
    assert!(matches!(
        opened,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let table = store(&writer, &availability);
    let request = FindServers {
        filter: None,
        descending: false,
        take: None,
    };
    let found = table
        .find_servers(&request)
        .await
        .expect_err("no pool means no rows");
    assert!(matches!(
        found,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let update = ServerUpdate {
        server_id: "s1".to_owned(),
        data: row(),
    };
    let updated = table
        .update_server(&update)
        .await
        .expect_err("no pool means no update");
    assert!(matches!(updated, McpTableError::Table(error) if no_database(&error)));
    let write = LockedWrite {
        server_name: Some("srv".to_owned()),
        alias: None,
        exclude_server_id: None,
        write: ServerWrite::Create(row()),
    };
    let locked = mcp_server_write_if_identifier_free(&table, &write)
        .await
        .err()
        .expect("no pool means no locks");
    assert!(matches!(
        locked,
        McpTableError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn user_table_helpers_without_a_pool_report_no_database() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let table = store(&writer, &availability);
    let credential = db_find_user_credential_row(&table, "u1", "s1")
        .await
        .expect_err("no pool means no credential");
    assert!(matches!(
        credential,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let credentials = db_find_user_credential_rows(&table, None)
        .await
        .expect_err("no pool means no credentials");
    assert!(matches!(
        credentials,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let env_vars = db_find_user_env_var_rows(&table, None)
        .await
        .expect_err("no pool means no environment");
    assert!(matches!(
        env_vars,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let upserted = db_upsert_user_credential_row(&table, "u1", "s1", "blob")
        .await
        .expect_err("no pool means no upsert");
    assert!(matches!(upserted, McpTableError::Table(error) if no_database(&error)));
}
