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
use crate::postgres_store::session::PostgresStore;

use super::super::store::{
    delete_user_credential, get_user_credential, get_user_oauth_credential, store_user_credential,
};
use super::super::types::{CredentialError, CredentialType, StoreOAuthCredential};
use super::super::{
    list_server_user_credentials, list_user_oauth_credentials, store_user_oauth_credential,
};
use super::support::{cipher, exposed, now};

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

async fn seeded() -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["mcp-user-credentials-database"])
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
    let store = PostgresStore { pool: Some(pool) };
    (fixture, store)
}

fn table<'db>(writer: &'db PostgresStore, availability: &'db Availability) -> McpTableStore<'db> {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    McpTableStore::new(Some(client), publisher()).expect("a client builds a store")
}

fn tokens() -> (SecretString, SecretString) {
    (SecretString::from("tok-abc"), SecretString::from("ref-1"))
}

fn oauth_request<'token>(
    token: &'token SecretString,
    refresh: &'token SecretString,
    skip_byok_guard: bool,
) -> StoreOAuthCredential<'token> {
    StoreOAuthCredential {
        user_id: "u1",
        server_id: "s1",
        access_token: token,
        refresh_token: Some(refresh),
        expires_in: Some(3600),
        scopes: None,
        skip_byok_guard,
        identity_binding_proof: None,
    }
}

#[tokio::test]
async fn a_byok_credential_round_trips_through_the_table() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let table = table(&writer, &availability);
    store_user_credential(&table, &cipher(), "u1", "s1", "secret-value")
        .await
        .expect("the store writes");
    let read = get_user_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs");
    assert_eq!(exposed(read.as_ref()), Some("secret-value"));
    let other = get_user_credential(&table, &cipher(), "u1", "s2")
        .await
        .expect("the read runs");
    assert!(other.is_none());
    delete_user_credential(&table, "u1", "s1")
        .await
        .expect("the delete runs");
    let gone = get_user_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs");
    assert!(gone.is_none());
}

#[tokio::test]
async fn a_readable_byok_key_is_protected_by_the_byok_guard() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let table = table(&writer, &availability);
    let (token, refresh) = tokens();
    store_user_credential(&table, &cipher(), "u1", "s1", "secret-value")
        .await
        .expect("the BYOK key stores");
    let refused = store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(&token, &refresh, false),
        now(),
    )
    .await
    .expect_err("the readable key is protected");
    assert!(matches!(
        refused,
        CredentialError::RefusesByokOverwrite { .. }
    ));
    let kept = get_user_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs");
    assert_eq!(exposed(kept.as_ref()), Some("secret-value"));
}

#[tokio::test]
async fn a_skipped_byok_guard_writes_the_oauth_grant_and_lists_it() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let table = table(&writer, &availability);
    let (token, refresh) = tokens();
    store_user_credential(&table, &cipher(), "u1", "s1", "secret-value")
        .await
        .expect("the BYOK key stores");
    store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(&token, &refresh, true),
        now(),
    )
    .await
    .expect("the skipped guard writes");
    let payload = get_user_oauth_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs")
        .expect("u1 holds an OAuth2 payload");
    assert_eq!(exposed(payload.access_token.as_ref()), Some("tok-abc"));
    assert_eq!(exposed(payload.refresh_token.as_ref()), Some("ref-1"));
    let items = list_server_user_credentials(&table, &cipher(), "s1")
        .await
        .expect("the listing runs");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].credential_type, CredentialType::Oauth2);
    assert!(items[0].expires_at.is_some());
    assert!(items[0].connected_at.is_some());
    assert!(!items[0].updated_at.is_empty());
    let listed = list_user_oauth_credentials(&table, &cipher(), "u1")
        .await
        .expect("the listing runs");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].server_id.as_deref(), Some("s1"));
}

#[tokio::test]
async fn an_undecryptable_row_is_replaced_by_a_new_oauth_grant() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let table = table(&writer, &availability);
    let (token, refresh) = tokens();
    db_upsert_user_credential_row(&table, "u1", "s1", "_w==")
        .await
        .expect("the unreadable row inserts");
    store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(&token, &refresh, false),
        now(),
    )
    .await
    .expect("the unreadable row is replaced");
    let payload = get_user_oauth_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs")
        .expect("u1 holds an OAuth2 payload");
    assert_eq!(exposed(payload.access_token.as_ref()), Some("tok-abc"));
    let items = list_server_user_credentials(&table, &cipher(), "s1")
        .await
        .expect("the listing runs");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].credential_type, CredentialType::Oauth2);
}

#[tokio::test]
async fn an_empty_stored_value_is_replaced_by_a_new_oauth_grant() {
    let (_fixture, writer) = seeded().await;
    let availability = Availability::default();
    let table = table(&writer, &availability);
    let (token, refresh) = tokens();
    store_user_credential(&table, &cipher(), "u1", "s1", "")
        .await
        .expect("the empty value stores");
    store_user_oauth_credential(
        &table,
        &cipher(),
        &oauth_request(&token, &refresh, false),
        now(),
    )
    .await
    .expect("the empty value is replaced");
    let payload = get_user_oauth_credential(&table, &cipher(), "u1", "s1")
        .await
        .expect("the read runs")
        .expect("u1 holds an OAuth2 payload");
    assert_eq!(exposed(payload.access_token.as_ref()), Some("tok-abc"));
}
