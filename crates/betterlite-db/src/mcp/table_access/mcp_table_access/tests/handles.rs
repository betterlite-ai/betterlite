use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::actions::{
    _mcp_server_table_actions, _oauth_client_table_actions, _user_credential_actions,
    _user_env_var_actions,
};
use crate::mcp::table_access::store::{_db_transaction_manager, McpTableStore};
use crate::mcp::table_access::types::McpTableError;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok(()) });
        future
    })
}

#[tokio::test]
async fn every_mcp_table_handle_needs_a_database_client() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    assert!(_mcp_server_table_actions(Some(client), publisher()).is_ok());
    assert!(_oauth_client_table_actions(Some(client), publisher()).is_ok());
    assert!(_user_credential_actions(Some(client), publisher()).is_ok());
    assert!(_user_env_var_actions(Some(client), publisher()).is_ok());
    let absent: Option<PrismaClient<'static>> = None;
    assert!(matches!(
        _mcp_server_table_actions(absent, publisher()),
        Err(McpTableError::Database(DbError::NoDbConnected))
    ));
}

#[tokio::test]
async fn a_store_without_a_pool_cannot_open_a_transaction() {
    let writer = PostgresStore { pool: None };
    let error = _db_transaction_manager(&writer)
        .await
        .err()
        .expect("no pool means no session");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let absent: Option<PrismaClient<'static>> = None;
    let error = McpTableStore::new(absent, publisher())
        .err()
        .expect("a store needs a client");
    assert!(matches!(
        error,
        McpTableError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn a_degraded_reader_falls_back_to_the_writer() {
    let writer = PostgresStore { pool: None };
    let reader = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &reader,
        availability: &availability,
    };
    let store = McpTableStore::new(Some(client), publisher())
        .ok()
        .expect("a client builds a store");
    assert!(std::ptr::eq(store.reader(), &reader));
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert!(std::ptr::eq(store.reader(), &writer));
}
