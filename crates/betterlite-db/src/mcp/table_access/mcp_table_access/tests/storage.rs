use std::sync::Arc;

use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_redis::config_sync::table_actions::TableActions as _;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::actions::_mcp_server_table_actions;
use crate::mcp::table_access::filter::{FindServers, ServerFilter};
use crate::mcp::table_access::rows::{
    CREDENTIAL_TABLE, KeyValues, RowFields, SERVER_TABLE, ServerUpdate,
};
use crate::mcp::table_access::sql::{self, Select};
use crate::mcp::table_access::store::{_db_transaction_manager, McpTableStore};
use crate::mcp::table_access::types::{LockedWrite, McpServerAccess, McpTableError, ServerWrite};
use crate::mcp::table_access::user_tables::{
    _db_find_user_credential_row, _db_find_user_credential_rows, _db_find_user_env_var_rows,
    _db_upsert_user_credential_row,
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
    McpTableStore::new(Some(client), publisher())
        .ok()
        .expect("a client builds a store")
}

fn row() -> RowFields {
    RowFields::texts([("server_id", "s1")])
}

#[tokio::test]
async fn sql_reads_without_a_pool_report_no_database() {
    let store = PostgresStore { pool: None };
    let binds = ["s1".to_owned()];
    let select = Select {
        predicate: Some("\"server_id\" = $1".to_owned()),
        binds: &binds,
        descending: false,
        take: None,
    };
    let error = sql::select(&store, &SERVER_TABLE, &select)
        .await
        .err()
        .expect("no pool means no rows");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let request = FindServers {
        filter: Some(ServerFilter::ServerIds(vec!["s1".to_owned()])),
        descending: false,
        take: None,
    };
    let error = sql::select_servers(&store, &SERVER_TABLE, &request)
        .await
        .err()
        .expect("no pool means no rows");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let error = sql::select_keyed(&store, &CREDENTIAL_TABLE, None)
        .await
        .err()
        .expect("no pool means no rows");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
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
    let table = _mcp_server_table_actions(Some(client), publisher())
        .ok()
        .expect("a client builds table actions");
    let error = table
        .create(row())
        .await
        .err()
        .expect("no pool means no insert");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let error = table
        .create_many(vec![row()])
        .await
        .err()
        .expect("no pool means no insert");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let filter = KeyValues::from([("server_id", "s1".to_owned())]);
    let error = table
        .update(row(), filter.clone())
        .await
        .err()
        .expect("no pool means no update");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let error = table
        .update_many(row(), filter.clone())
        .await
        .err()
        .expect("no pool means no update");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let error = table
        .upsert(KeyValues::new(), row())
        .await
        .err()
        .expect("no pool means no upsert");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let error = table
        .delete(filter.clone())
        .await
        .err()
        .expect("no pool means no delete");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
    let error = table
        .delete_many(filter)
        .await
        .err()
        .expect("no pool means no delete");
    assert!(matches!(
        error,
        TableError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn a_pool_less_store_reports_no_database_for_reads_updates_and_locked_writes() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let error = _db_transaction_manager(&writer)
        .await
        .err()
        .expect("no pool means no session");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let store = store(&writer, &availability);
    let request = FindServers {
        filter: None,
        descending: false,
        take: None,
    };
    let error = store
        .find_servers(&request)
        .await
        .err()
        .expect("no pool means no rows");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let update = ServerUpdate {
        server_id: "s1".to_owned(),
        data: row(),
    };
    let error = store
        .update_server(&update)
        .await
        .err()
        .expect("no pool means no update");
    assert!(matches!(
        error,
        McpTableError::Table(TableError::Database(DbError::NoDbConnected))
    ));
    let write = LockedWrite {
        server_name: Some("srv".to_owned()),
        alias: None,
        exclude_server_id: None,
        write: ServerWrite::Create(row()),
    };
    let error = store
        .write_if_identifier_free(&write)
        .await
        .err()
        .expect("no pool means no locks");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn user_table_helpers_without_a_pool_report_no_database() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let store = store(&writer, &availability);
    let error = _db_find_user_credential_row(&store, "u1", "s1")
        .await
        .err()
        .expect("no pool means no credential");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let error = _db_find_user_credential_rows(&store, None)
        .await
        .err()
        .expect("no pool means no credentials");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let error = _db_find_user_env_var_rows(&store, None)
        .await
        .err()
        .expect("no pool means no environment");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let error = _db_upsert_user_credential_row(&store, "u1", "s1", "blob")
        .await
        .err()
        .expect("no pool means no upsert");
    assert!(matches!(
        error,
        McpTableError::Table(TableError::Database(DbError::NoDbConnected))
    ));
}
