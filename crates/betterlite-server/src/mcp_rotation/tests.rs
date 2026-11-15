#![cfg(test)]

mod blob;
mod codec;
mod env_vars;
mod rotation;
mod support;

use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use self::support::{NEW_KEY, OLD_KEY, cipher, new_key, open, sealed, string};
use super::codec::encrypt_credentials;
use super::postgres::PostgresRotationStore;
use super::rotation::Rotation;

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB,
    env_vars JSONB,
    static_headers JSONB,
    env JSONB,
    updated_by TEXT
);
CREATE TABLE "LiteLLM_MCPServerOAuthClient" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB
);
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    credential_b64 TEXT NOT NULL,
    UNIQUE (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    values_b64 TEXT NOT NULL,
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

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["mcp-rotation-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
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

async fn column(client: &Client, sql: &str) -> Option<String> {
    client.query_one(sql, &[]).await.unwrap().get(0)
}

async fn execute(
    client: &Client,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) {
    client.execute(sql, params).await.unwrap();
}

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn credential_field(column: &str, field: &str) -> String {
    let JsonValue::Object(fields) = json(column) else {
        panic!("the rotated credentials are a JSON object");
    };
    let Some(JsonValue::String(ciphertext)) = fields.get(field) else {
        panic!("{field} is a ciphertext string");
    };
    ciphertext.clone()
}

#[tokio::test]
async fn postgres_rows_and_failures() {
    let (fixture, pool, client) = database().await;
    let blob = encrypted_blob_text();
    let env_vars = env_vars_text();
    let mut headers_map = SecretMap::new();
    headers_map.insert("X-Key".to_owned(), SecretString::from("plain-value"));
    let headers = encrypt_secret_map(&cipher(OLD_KEY), &headers_map, None).unwrap();
    let user_token = sealed("user-token", OLD_KEY);
    let user_env = sealed("user-env", OLD_KEY);
    execute(
        &client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, credentials, env_vars, static_headers) VALUES ('s1', $1::jsonb, to_jsonb($2::text), to_jsonb($3::text))"#,
        &[&blob, &env_vars, &headers],
    )
    .await;
    execute(
        &client,
        r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, credentials) VALUES ('boom', $1::jsonb)"#,
        &[&blob],
    )
    .await;
    execute(
        &client,
        r#"INSERT INTO "LiteLLM_MCPServerOAuthClient" (server_id, credentials) VALUES ('oauth-1', $1::jsonb), ('oauth-2', $1::jsonb)"#,
        &[&blob],
    )
    .await;
    execute(
        &client,
        r#"INSERT INTO "LiteLLM_MCPUserCredentials" (id, user_id, server_id, credential_b64) VALUES ('c1', 'u1', 's1', $1), ('c2', 'u2', 's1', 'AA')"#,
        &[&user_token],
    )
    .await;
    execute(
        &client,
        r#"INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64) VALUES ('e1', 'u1', 's1', $1), ('e2', 'u2', 's1', 'AA')"#,
        &[&user_env],
    )
    .await;
    let store = PostgresRotationStore {
        store: PostgresStore {
            pool: Some(pool.clone()),
        },
    };
    let old = cipher(OLD_KEY);
    let key = new_key();
    let result = Rotation {
        store: &store,
        cipher: &old,
        new_key: &key,
    }
    .servers("rotator")
    .await;
    assert!(result.is_err(), "the boom row aborts the server stage");
    let rotated = column(
        &client,
        r#"SELECT credentials::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#,
    )
    .await
    .unwrap();
    let client_id = credential_field(&rotated, "client_id");
    assert_eq!(
        open(&client_id, NEW_KEY).as_deref(),
        Some("client-id-value")
    );
    let updated_by = column(
        &client,
        r#"SELECT updated_by FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#,
    )
    .await
    .unwrap();
    assert_eq!(updated_by, "rotator");
    let env_vars = column(
        &client,
        r#"SELECT env_vars::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#,
    )
    .await
    .unwrap();
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
    let headers = column(
        &client,
        r#"SELECT static_headers::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 's1'"#,
    )
    .await
    .unwrap();
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
            .map(|value| value.expose_secret().to_owned()),
        Some("plain-value".to_owned())
    );
    let boom = column(
        &client,
        r#"SELECT credentials::text FROM "LiteLLM_MCPServerTable" WHERE server_id = 'boom'"#,
    )
    .await
    .unwrap();
    let boom_id = credential_field(&boom, "client_id");
    assert_eq!(open(&boom_id, OLD_KEY).as_deref(), Some("client-id-value"));
    assert_eq!(open(&boom_id, NEW_KEY), None);
    let oauth = column(
        &client,
        r#"SELECT credentials::text FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = 'oauth-1'"#,
    )
    .await
    .unwrap();
    let oauth_id = credential_field(&oauth, "client_id");
    assert_eq!(open(&oauth_id, OLD_KEY).as_deref(), Some("client-id-value"));
    client
        .batch_execute(r#"DROP TRIGGER reject_boom_update ON "LiteLLM_MCPServerTable""#)
        .await
        .unwrap();
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &key,
    }
    .servers("rotator")
    .await
    .unwrap();
    for server_id in ["oauth-1", "oauth-2"] {
        let oauth = column(
            &client,
            &format!(
                r#"SELECT credentials::text FROM "LiteLLM_MCPServerOAuthClient" WHERE server_id = '{server_id}'"#
            ),
        )
        .await
        .unwrap();
        let field = credential_field(&oauth, "client_id");
        assert_eq!(open(&field, NEW_KEY).as_deref(), Some("client-id-value"));
    }
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &key,
    }
    .user_credentials()
    .await
    .unwrap();
    let credentials = column(
        &client,
        r#"SELECT credential_b64 FROM "LiteLLM_MCPUserCredentials" WHERE user_id = 'u1' AND server_id = 's1'"#,
    )
    .await
    .unwrap();
    assert_eq!(
        open(credentials.as_deref().unwrap(), NEW_KEY).as_deref(),
        Some("user-token")
    );
    let skipped = column(
        &client,
        r#"SELECT credential_b64 FROM "LiteLLM_MCPUserCredentials" WHERE user_id = 'u2' AND server_id = 's1'"#,
    )
    .await
    .unwrap();
    assert_eq!(skipped.as_deref(), Some("AA"));
    Rotation {
        store: &store,
        cipher: &old,
        new_key: &key,
    }
    .user_env_vars()
    .await
    .unwrap();
    let values = column(
        &client,
        r#"SELECT values_b64 FROM "LiteLLM_MCPUserEnvVars" WHERE user_id = 'u1' AND server_id = 's1'"#,
    )
    .await
    .unwrap();
    assert_eq!(
        open(values.as_deref().unwrap(), NEW_KEY).as_deref(),
        Some("user-env")
    );
    let skipped = column(
        &client,
        r#"SELECT values_b64 FROM "LiteLLM_MCPUserEnvVars" WHERE user_id = 'u2' AND server_id = 's1'"#,
    )
    .await
    .unwrap();
    assert_eq!(skipped.as_deref(), Some("AA"));
    drop(client);
    pool.close();
    fixture.close_async().await.unwrap();
}
