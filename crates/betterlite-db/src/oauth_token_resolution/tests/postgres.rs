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
use crate::mcp::table_access::user_tables::db_upsert_user_credential_row;
use crate::mcp_user_credentials::rows::{CredentialRows as _, CredentialScope};
use crate::oauth_token_resolution::purge::PurgeRows as _;
use crate::postgres_store::session::PostgresStore;

const STORAGE: &str = r#"
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    id text PRIMARY KEY,
    user_id text NOT NULL,
    server_id text NOT NULL,
    credential_b64 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, server_id)
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["oauth-token-resolution-database"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(configured.as_ref(), vec![STORAGE.to_owned()])
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

fn store<'db>(writer: &'db PostgresStore, availability: &'db Availability) -> McpTableStore<'db> {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    McpTableStore::new(Some(client), publisher()).expect("a client builds a store")
}

#[tokio::test]
async fn pairs_are_deleted_for_the_enumerated_users_of_the_server() {
    let (_fixture, writer) = database().await;
    let availability = Availability::default();
    let table = store(&writer, &availability);
    db_upsert_user_credential_row(&table, "u1", "s1", "row-1")
        .await
        .expect("the row inserts");
    db_upsert_user_credential_row(&table, "u2", "s1", "row-2")
        .await
        .expect("the row inserts");
    db_upsert_user_credential_row(&table, "u1", "s2", "row-3")
        .await
        .expect("the row inserts");
    let users = ["u1".to_owned(), "u2".to_owned()];
    let deleted = table
        .delete_credential_pairs("s1", &users)
        .await
        .expect("the pairs delete");
    assert_eq!(deleted, 2);
    let remaining = table
        .find_credentials(CredentialScope::Server("s1"))
        .await
        .expect("the read runs");
    assert!(remaining.is_empty());
    let other = table
        .find_credentials(CredentialScope::Server("s2"))
        .await
        .expect("the read runs");
    assert_eq!(other.len(), 1);
}
