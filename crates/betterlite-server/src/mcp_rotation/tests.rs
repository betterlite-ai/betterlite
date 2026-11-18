#![cfg(test)]

mod blob;
mod codec;
mod env_vars;
mod rotation;
mod support;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::auxiliary_repository::bindings::{
    MCPServerOAuthClient, MCPServerTable, MCPUserCredentials,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_redis::config_sync::table_actions::TableActions as _;
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use self::support::{NEW_KEY, OLD_KEY, TestCipher, cipher, new_key, open, sealed, string};
use super::codec::encrypt_credentials;
use super::rotation::Rotation;
use super::tables::{McpUpdateActions, McpUserEnvVars, PostgresRotationStore};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB,
    env_vars JSONB,
    static_headers JSONB,
    env JSONB,
    updated_by TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT '2000-01-01'
);
CREATE TABLE "LiteLLM_MCPServerOAuthClient" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB,
    updated_at TIMESTAMP NOT NULL DEFAULT '2000-01-01'
);
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    credential_b64 TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT '2000-01-01',
    UNIQUE (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    values_b64 TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT '2000-01-01',
    UNIQUE (user_id, server_id)
);
CREATE FUNCTION reject_boom() RETURNS trigger AS $$
BEGIN
    IF NEW.server_id = 'boom' THEN
        RAISE EXCEPTION 'the boom row cannot be updated';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER reject_boom_update BEFORE UPDATE ON "LiteLLM_MCPServerTable"
FOR EACH ROW EXECUTE FUNCTION reject_boom();
"#;

const INSERT_SERVER_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, credentials, env_vars, static_headers) VALUES ('s1', to_jsonb($1::text), to_jsonb($2::text), to_jsonb($3::text))"#;
const INSERT_BOOM_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, credentials) VALUES ('boom', to_jsonb($1::text))"#;
const INSERT_OAUTH_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerOAuthClient" (server_id, credentials) VALUES ('oauth-1', to_jsonb($1::text)), ('oauth-2', to_jsonb($1::text))"#;
const INSERT_CREDENTIAL_SQL: &str = r#"INSERT INTO "LiteLLM_MCPUserCredentials" (id, user_id, server_id, credential_b64) VALUES ('c1', 'u1', 's1', $1), ('c2', 'u2', 's1', 'AA')"#;
const INSERT_ENV_VAR_SQL: &str = r#"INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64) VALUES ('e1', 'u1', 's1', $1), ('e2', 'u2', 's1', 'AA')"#;
const SERVER_CREDENTIALS_SQL: &str =
    r#"SELECT credentials::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#;
const SERVER_UPDATED_BY_SQL: &str =
    r#"SELECT updated_by FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#;
const SERVER_ENV_VARS_SQL: &str =
    r#"SELECT env_vars::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#;
const SERVER_HEADERS_SQL: &str =
    r#"SELECT static_headers::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#;
const BOOM_CREDENTIALS_SQL: &str =
    r#"SELECT credentials::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 'boom'"#;
const OAUTH_CREDENTIALS_SQL: &str =
    r#"SELECT credentials::text FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = $1"#;
const USER_CREDENTIAL_SQL: &str = r#"SELECT credential_b64 FROM "LiteLLM_MCPUserCredentials" WHERE user_id = $1 AND server_id = 's1'"#;
const USER_ENV_SQL: &str =
    r#"SELECT values_b64 FROM "LiteLLM_MCPUserEnvVars" WHERE user_id = $1 AND server_id = 's1'"#;
const TOUCHED_SQL: &str = r#"SELECT
    (SELECT updated_at > '2001-01-01' FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'),
    (SELECT updated_at > '2001-01-01' FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = 'oauth-1'),
    (SELECT updated_at > '2001-01-01' FROM "LiteLLM_MCPUserCredentials" WHERE id = 'c1'),
    (SELECT updated_at > '2001-01-01' FROM "LiteLLM_MCPUserCredentials" WHERE id = 'c2'),
    (SELECT updated_at > '2001-01-01' FROM "LiteLLM_MCPUserEnvVars" WHERE id = 'e1')"#;
const USER_ENV_VAR_UPDATE_SQL: &str =
    r#"UPDATE "LiteLLM_MCPUserEnvVars" SET values_b64 = $3 WHERE user_id = $1 AND server_id = $2"#;
const DROP_TRIGGER_SQL: &str = r#"DROP TRIGGER reject_boom_update ON "LiteLLM_MCPServerTable""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["mcp-rotation-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    let client = pool.get().await.unwrap();
    (fixture, pool, client)
}

fn encrypted_blob_text() -> String {
    let mut fields = CredentialBlob::new();
    fields.insert("client_id".to_owned(), string("client-id-value"));
    fields.insert("client_secret".to_owned(), string("client-secret-value"));
    encrypt_credentials(&mut fields, &cipher(OLD_KEY), None).unwrap();
    serde_json::to_string(&JsonValue::Object(fields)).unwrap()
}

fn env_vars_text() -> String {
    serde_json::to_string(&JsonValue::Array(vec![JsonValue::Object(
        [
            ("name".to_owned(), string("token")),
            ("value".to_owned(), string(&sealed("global-value", OLD_KEY))),
            ("scope".to_owned(), string("global")),
        ]
        .into_iter()
        .collect(),
    )]))
    .unwrap()
}

struct Texts {
    blob: String,
    env_vars: String,
    headers: String,
    user_token: String,
    user_env: String,
}

fn texts() -> Texts {
    let mut headers_map = SecretMap::new();
    headers_map.insert("X-Key".to_owned(), SecretString::from("plain-value"));
    Texts {
        blob: encrypted_blob_text(),
        env_vars: env_vars_text(),
        headers: encrypt_secret_map(&cipher(OLD_KEY), &headers_map, None).unwrap(),
        user_token: sealed("user-token", OLD_KEY),
        user_env: sealed("user-env", OLD_KEY),
    }
}

async fn column(client: &Client, sql: &str) -> Option<String> {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn column_param(client: &Client, sql: &str, param: &str) -> Option<String> {
    client.query_one(sql, &[&param]).await.unwrap().get(0)
}

async fn execute(client: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    client.execute(sql, params).await.unwrap();
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn credential_field(column: &str, field: &str) -> String {
    let stored = match json(column) {
        JsonValue::String(text) => json(&text),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => other,
    };
    let JsonValue::Object(fields) = stored else {
        panic!("the rotated credentials are a JSON object");
    };
    let Some(JsonValue::String(ciphertext)) = fields.get(field) else {
        panic!("{field} is a ciphertext string");
    };
    ciphertext.clone()
}

async fn seed(client: &Client, texts: &Texts) {
    execute(
        client,
        INSERT_SERVER_SQL,
        &[&texts.blob, &texts.env_vars, &texts.headers],
    )
    .await;
    execute(client, INSERT_BOOM_SQL, &[&texts.blob]).await;
    execute(client, INSERT_OAUTH_SQL, &[&texts.blob]).await;
    execute(client, INSERT_CREDENTIAL_SQL, &[&texts.user_token]).await;
    execute(client, INSERT_ENV_VAR_SQL, &[&texts.user_env]).await;
}

async fn assert_server_rotated(client: &Client) {
    let rotated = column(client, SERVER_CREDENTIALS_SQL).await.unwrap();
    let client_id = credential_field(&rotated, "client_id");
    assert_eq!(
        open(&client_id, NEW_KEY).as_deref(),
        Some("client-id-value")
    );
    let updated_by = column(client, SERVER_UPDATED_BY_SQL).await.unwrap();
    assert_eq!(updated_by, "rotator");
    let env_vars = column(client, SERVER_ENV_VARS_SQL).await.unwrap();
    let JsonValue::String(inner) = json(&env_vars) else {
        panic!("the rotated env var column is a JSON string");
    };
    let JsonValue::Array(entries) = json(&inner) else {
        panic!("the rotated env var text is a JSON array");
    };
    let Some(JsonValue::Object(entry)) = entries.first() else {
        panic!("a rotated env entry is an object");
    };
    let Some(JsonValue::String(value)) = entry.get("value") else {
        panic!("the env value is a ciphertext string");
    };
    assert_eq!(open(value, NEW_KEY).as_deref(), Some("global-value"));
    let headers = column(client, SERVER_HEADERS_SQL).await.unwrap();
    let JsonValue::String(stored_headers) = json(&headers) else {
        panic!("the static headers column is a JSON string");
    };
    let map = decode_secret_map(
        &cipher(NEW_KEY),
        Some(stored_headers.as_str()),
        "static_headers",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        map.get("X-Key")
            .map(|secret| secret.expose_secret().to_owned()),
        Some("plain-value".to_owned())
    );
}

async fn assert_boom_untouched(client: &Client) {
    let boom = column(client, BOOM_CREDENTIALS_SQL).await.unwrap();
    let boom_id = credential_field(&boom, "client_id");
    assert_eq!(open(&boom_id, OLD_KEY).as_deref(), Some("client-id-value"));
    assert_eq!(open(&boom_id, NEW_KEY), None);
}

async fn assert_oauth(client: &Client, server_id: &str, key: &str) {
    let stored = column_param(client, OAUTH_CREDENTIALS_SQL, server_id)
        .await
        .unwrap();
    let field = credential_field(&stored, "client_id");
    assert_eq!(open(&field, key).as_deref(), Some("client-id-value"));
}

fn recording_publisher(journal: &Arc<Mutex<Vec<String>>>) -> PublishChange<TableError> {
    let entries = Arc::clone(journal);
    Arc::new(move |object_type: &str| {
        entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(object_type.to_owned());
        let future: PublishFuture<TableError> = Box::pin(async { Ok(()) });
        future
    })
}

fn published(journal: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    journal
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

// Prisma stamps updated_at on every update, so each rotated row moves past its seeded date.
async fn assert_touched(client: &Client) {
    let row = client.query_one(TOUCHED_SQL, &[]).await.unwrap();
    let touched: Vec<bool> = (0_usize..5).map(|index| row.get(index)).collect();
    assert_eq!(touched, [true, true, true, false, true]);
}

fn keys(user_id: &str, server_id: &str) -> Vec<String> {
    vec![user_id.to_owned(), server_id.to_owned()]
}

// The one-statement actions report whether a row matched and how many rows changed.
async fn assert_update_results(writer: &PostgresStore) {
    let actions = McpUpdateActions {
        access: TableAccess {
            read: writer,
            write: writer,
        },
        update_sql: USER_ENV_VAR_UPDATE_SQL,
    };
    let data = vec![Some("replacement".to_owned())];
    let hit = actions.update(data.clone(), keys("u1", "s1")).await;
    assert_eq!(hit.unwrap(), Some(()));
    let miss = actions.update(data.clone(), keys("nobody", "s1")).await;
    assert_eq!(miss.unwrap(), None);
    let many = actions.update_many(data.clone(), keys("u1", "s1")).await;
    assert_eq!(many.unwrap(), 1);
    let none = actions.update_many(data, keys("nobody", "s1")).await;
    assert_eq!(none.unwrap(), 0);
}

#[tokio::test]
async fn table_actions_only_update() {
    let store = PostgresStore { pool: None };
    let actions = McpUpdateActions {
        access: TableAccess {
            read: &store,
            write: &store,
        },
        update_sql: USER_ENV_VAR_UPDATE_SQL,
    };
    assert!(matches!(
        actions.create(()).await,
        Err(TableError::MissingRow)
    ));
    assert!(matches!(
        actions.create_many(vec![()]).await,
        Err(TableError::MissingRow)
    ));
    assert!(matches!(
        actions.upsert(keys("u", "s"), ()).await,
        Err(TableError::MissingRow)
    ));
    assert!(matches!(
        actions.delete(keys("u", "s")).await,
        Err(TableError::MissingRow)
    ));
    assert!(matches!(
        actions.delete_many(keys("u", "s")).await,
        Err(TableError::MissingRow)
    ));
    actions
        .update(Vec::new(), keys("u", "s"))
        .await
        .unwrap_err();
    actions
        .update_many(Vec::new(), keys("u", "s"))
        .await
        .unwrap_err();
}

async fn user_stages(
    client: &Client,
    store: &PostgresRotationStore<'_>,
    old: &TestCipher,
    key: &SecretString,
) {
    Rotation {
        store,
        cipher: old,
        new_key: key,
    }
    .user_credentials()
    .await
    .unwrap();
    let rotated = column_param(client, USER_CREDENTIAL_SQL, "u1")
        .await
        .unwrap();
    assert_eq!(open(&rotated, NEW_KEY).as_deref(), Some("user-token"));
    let skipped = column_param(client, USER_CREDENTIAL_SQL, "u2").await;
    assert_eq!(skipped.as_deref(), Some("AA"));
    Rotation {
        store,
        cipher: old,
        new_key: key,
    }
    .user_env_vars()
    .await
    .unwrap();
    let values = column_param(client, USER_ENV_SQL, "u1").await.unwrap();
    assert_eq!(open(&values, NEW_KEY).as_deref(), Some("user-env"));
    let env_skipped = column_param(client, USER_ENV_SQL, "u2").await;
    assert_eq!(env_skipped.as_deref(), Some("AA"));
}

fn rotation_store<'db>(
    writer: &'db PostgresStore,
    availability: &'db Availability,
    journal: &Arc<Mutex<Vec<String>>>,
) -> PostgresRotationStore<'db> {
    let prisma = Some(PrismaClient {
        writer,
        reader: writer,
        availability,
    });
    let publish = recording_publisher(journal);
    PostgresRotationStore {
        servers: PrismaTableRepository::new(prisma, Arc::clone(&publish), MCPServerTable),
        oauth_clients: PrismaTableRepository::new(
            prisma,
            Arc::clone(&publish),
            MCPServerOAuthClient,
        ),
        user_credentials: PrismaTableRepository::new(
            prisma,
            Arc::clone(&publish),
            MCPUserCredentials,
        ),
        user_env_vars: PrismaTableRepository::new(prisma, publish, McpUserEnvVars),
    }
}

async fn server_stages(
    client: &Client,
    store: &PostgresRotationStore<'_>,
    old: &TestCipher,
    key: &SecretString,
    journal: &Arc<Mutex<Vec<String>>>,
) {
    let abort = Rotation {
        store,
        cipher: old,
        new_key: key,
    }
    .servers("rotator")
    .await;
    assert!(abort.is_err(), "the boom row aborts the server stage");
    assert_eq!(
        published(journal),
        ["litellm_mcpservertable"],
        "only the committed server write is published"
    );
    assert_server_rotated(client).await;
    assert_boom_untouched(client).await;
    assert_oauth(client, "oauth-1", OLD_KEY).await;
    client.batch_execute(DROP_TRIGGER_SQL).await.unwrap();
    Rotation {
        store,
        cipher: old,
        new_key: key,
    }
    .servers("rotator")
    .await
    .unwrap();
    assert_oauth(client, "oauth-1", NEW_KEY).await;
    assert_oauth(client, "oauth-2", NEW_KEY).await;
}

#[tokio::test]
async fn postgres_rows_and_failures() {
    let (fixture, pool, client) = database().await;
    seed(&client, &texts()).await;
    let writer = PostgresStore {
        pool: Some(pool.clone()),
    };
    let availability = Availability::default();
    let journal = Arc::new(Mutex::new(Vec::new()));
    let store = rotation_store(&writer, &availability, &journal);
    let old = cipher(OLD_KEY);
    let key = new_key();
    server_stages(&client, &store, &old, &key, &journal).await;
    user_stages(&client, &store, &old, &key).await;
    assert_eq!(
        published(&journal),
        ["litellm_mcpservertable"; 3],
        "the OAuth client and user tables are not config-synced"
    );
    assert_touched(&client).await;
    assert_update_results(&writer).await;
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
