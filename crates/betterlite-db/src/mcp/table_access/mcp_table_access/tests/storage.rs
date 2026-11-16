use std::fmt::Debug;
use std::sync::Arc;

use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::actions::mcp_server_table_actions;
use crate::mcp::table_access::filter::FindServers;
use crate::mcp::table_access::lock::mcp_server_write_if_identifier_free;
use crate::mcp::table_access::rows::{CREDENTIAL_TABLE, KeyValues, RowFields, ServerUpdate};
use crate::mcp::table_access::sql;
use crate::mcp::table_access::store::{McpTableStore, db_transaction_manager};
use crate::mcp::table_access::types::{
    LockedWrite, McpServerAccess as _, McpTableError, ServerWrite,
};
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

fn missing_pool(error: &McpTableError) -> bool {
    matches!(error, McpTableError::Database(DbError::NoDbConnected))
}

fn assert_no_database<T>(result: Result<T, TableError>)
where
    T: Debug,
{
    let error = result.expect_err("no pool means no write");
    assert!(no_database(&error));
}

#[tokio::test]
async fn sql_reads_without_a_pool_report_no_database() {
    let pool_less = PostgresStore { pool: None };
    let keyed = sql::select_keyed(&pool_less, &CREDENTIAL_TABLE, None)
        .await
        .expect_err("no pool means no rows");
    assert!(missing_pool(&keyed));
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
    assert_no_database(table.create(row()).await);
    assert_no_database(table.create_many(vec![row()]).await);
    assert_no_database(table.update(row(), filter.clone()).await);
    assert_no_database(table.update_many(row(), filter.clone()).await);
    assert_no_database(table.upsert(KeyValues::new(), row()).await);
    assert_no_database(table.delete(filter.clone()).await);
    assert_no_database(table.delete_many(filter).await);
}

#[tokio::test]
async fn a_pool_less_store_reports_no_database_for_reads_updates_and_locked_writes() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let opened = db_transaction_manager(&writer)
        .await
        .err()
        .expect("no pool means no session");
    assert!(missing_pool(&opened));
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
    assert!(missing_pool(&found));
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
    assert!(missing_pool(&locked));
}

#[tokio::test]
async fn user_table_helpers_without_a_pool_report_no_database() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let table = store(&writer, &availability);
    let credential = db_find_user_credential_row(&table, "u1", "s1")
        .await
        .expect_err("no pool means no credential");
    assert!(missing_pool(&credential));
    let credentials = db_find_user_credential_rows(&table, None)
        .await
        .expect_err("no pool means no credentials");
    assert!(missing_pool(&credentials));
    let env_vars = db_find_user_env_var_rows(&table, None)
        .await
        .expect_err("no pool means no environment");
    assert!(missing_pool(&env_vars));
    let upserted = db_upsert_user_credential_row(&table, "u1", "s1", "blob")
        .await
        .expect_err("no pool means no upsert");
    assert!(matches!(upserted, McpTableError::Table(error) if no_database(&error)));
}
