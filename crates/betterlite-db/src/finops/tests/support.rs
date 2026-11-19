#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret, SecretString};
use tokio::task::yield_now;
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::PrismaClient;
use crate::budget_permission_rows::JsonData;
use crate::database_replica::availability::Availability;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;
use crate::stored_cipher::DbCipher;

const POOL_SIZE: usize = 2;
const CIPHER_KEY: &str = "b3075-finops-settings-salt";

const STRING_ROW_INSERT: &str =
    r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value") VALUES ($1, to_jsonb($2::text))"#;
const MAPPING_ROW_INSERT: &str =
    r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value") VALUES ($1, $2::text::jsonb)"#;
const NULL_ROW_INSERT: &str =
    r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value") VALUES ($1, NULL)"#;
const READ_ROW_SQL: &str = r#"SELECT jsonb_typeof("param_value"), "param_value" #>> '{}'
FROM "LiteLLM_Config" WHERE "param_name" = $1"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(crate) async fn provision() -> Self {
        let url = Environment::parse_from(["finops-settings-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(POOL_SIZE)
            .build()
            .unwrap();
        Self {
            fixture,
            store: PostgresStore { pool: Some(pool) },
            availability: Availability::default(),
        }
    }

    pub(crate) const fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(crate) async fn close(self) {
        self.store.pool.unwrap().close();
        // the closed pool's drivers need a turn to close their sockets before DROP DATABASE
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

pub(crate) fn cipher() -> DbCipher {
    cipher_with_key(CIPHER_KEY)
}

pub(crate) fn cipher_with_key(key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn cipher_without_key() -> DbCipher {
    let mut cipher = cipher_with_key(CIPHER_KEY);
    cipher.settings.signing_key = None;
    cipher
}

pub(crate) fn seal(text: &str) -> String {
    seal_with(text, CIPHER_KEY)
}

pub(crate) fn seal_with(text: &str, key: &str) -> String {
    let sealed = cipher_with_key(key)
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .unwrap();
    let StoredValue::Text(value) = sealed else {
        panic!("a text value stays text");
    };
    value.expose_secret().to_owned()
}

pub(crate) fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

pub(crate) fn fields(entries: &[(&str, JsonData)]) -> BTreeMap<String, JsonData> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn json_object(entries: &[(&str, JsonData)]) -> String {
    serde_json::to_string(&fields(entries)).unwrap()
}

pub(crate) struct StoredRow {
    pub(crate) kind: String,
    pub(crate) text: String,
}

impl StoredRow {
    pub(crate) fn fields(&self) -> BTreeMap<String, JsonData> {
        serde_json::from_str(&self.text).unwrap()
    }
}

pub(crate) async fn seed_string(database: &Database, param: &str, text: &str) {
    execute(database, STRING_ROW_INSERT, &[&param, &text]).await;
}

pub(crate) async fn seed_mapping(database: &Database, param: &str, json: &str) {
    execute(database, MAPPING_ROW_INSERT, &[&param, &json]).await;
}

pub(crate) async fn seed_null(database: &Database, param: &str) {
    execute(database, NULL_ROW_INSERT, &[&param]).await;
}

pub(crate) async fn read_row(database: &Database, param: &str) -> Option<StoredRow> {
    let session = database.store.session().await.unwrap();
    let row = session.query_opt(READ_ROW_SQL, &[&param]).await.unwrap()?;
    Some(StoredRow {
        kind: row.get(0),
        text: row.get(1),
    })
}

async fn execute(database: &Database, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    database
        .store
        .session()
        .await
        .unwrap()
        .execute(sql, params)
        .await
        .unwrap();
}
