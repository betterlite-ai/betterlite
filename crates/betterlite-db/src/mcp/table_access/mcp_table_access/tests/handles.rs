use std::ptr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp::table_access::actions::{
    mcp_server_table_actions, oauth_client_table_actions, user_credential_actions,
    user_env_var_actions,
};
use crate::mcp::table_access::rows::{
    CREDENTIAL_TABLE, ENV_VAR_TABLE, OAUTH_CLIENT_TABLE, SERVER_TABLE,
};
use crate::mcp::table_access::store::McpTableStore;
use crate::mcp::table_access::types::McpTableError;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

#[test]
fn every_mcp_table_handle_binds_its_own_table() {
    let writer = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let server = mcp_server_table_actions(Some(client), publisher()).expect("a server handle");
    assert_eq!(server.actions.table.relation, SERVER_TABLE.relation);
    assert!(ptr::eq(server.actions.store, &writer));
    let oauth = oauth_client_table_actions(Some(client), publisher()).expect("an oauth handle");
    assert_eq!(oauth.actions.table.relation, OAUTH_CLIENT_TABLE.relation);
    let credentials =
        user_credential_actions(Some(client), publisher()).expect("a credential handle");
    assert_eq!(
        credentials.actions.table.relation,
        CREDENTIAL_TABLE.relation
    );
    let env_vars = user_env_var_actions(Some(client), publisher()).expect("an env-var handle");
    assert_eq!(env_vars.actions.table.relation, ENV_VAR_TABLE.relation);
    assert_eq!(env_vars.actions.table.name, "litellm_mcpuserenvvars");
}

#[test]
fn every_mcp_table_handle_needs_a_database_client() {
    let absent: Option<PrismaClient<'static>> = None;
    let missing = mcp_server_table_actions(absent, publisher())
        .err()
        .expect("no client means no handle");
    assert!(matches!(
        missing,
        McpTableError::Database(DbError::NoDbConnected)
    ));
    let unbuilt = McpTableStore::new(absent, publisher())
        .err()
        .expect("a store needs a client");
    assert!(matches!(
        unbuilt,
        McpTableError::Database(DbError::NoDbConnected)
    ));
}

#[test]
fn a_degraded_reader_falls_back_to_the_writer() {
    let writer = PostgresStore { pool: None };
    let reader = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &reader,
        availability: &availability,
    };
    let store = McpTableStore::new(Some(client), publisher()).expect("a client builds a store");
    assert!(ptr::eq(store.reader(), &reader));
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert!(ptr::eq(store.reader(), &writer));
}
