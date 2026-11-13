use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::types::ToSql;
use tokio_postgres::NoTls;

use super::super::fixtures::{decrypted_leaves, json, MASTER_KEY, PREVIOUS_KEY};
use crate::database_reencryption::outcome::{MigrationFailure, MigrationOutcome};
use crate::database_reencryption::registry::SecretColumn;
use crate::database_reencryption::runner::{migrate_if_requested, MigrationRequest};
use crate::encryption_scan::secret_text::SecretText;
use crate::encryption_scan::MigrationJson;
use crate::stored_cipher::DbCipher;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
    pub(crate) client: Client,
}

pub(crate) async fn database() -> Database {
    let url = Environment::parse_from(["database-reencryption-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .expect("the fixture provisions a schema");
    let configuration = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the fixture URL parses");
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("the pool builds");
    let client = pool.get().await.expect("a pooled session");
    Database {
        fixture,
        pool,
        client,
    }
}

impl Database {
    pub(crate) async fn close(self) {
        drop(self.client);
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }

    pub(crate) async fn execute(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .expect("the fixture SQL runs");
    }

    pub(crate) async fn create_table(&self, column: &SecretColumn) {
        let value_type = if column.is_json { "jsonb" } else { "text" };
        self.execute(&format!(
            "CREATE TABLE \"{}\" (\"{}\" text PRIMARY KEY, \"{}\" {value_type})",
            column.table, column.primary_key, column.column
        ))
        .await;
    }

    pub(crate) async fn insert(&self, column: &SecretColumn, key: &str, value: &str) {
        let cast = if column.is_json { "::jsonb" } else { "" };
        let sql = format!(
            "INSERT INTO \"{}\" (\"{}\", \"{}\") VALUES ($1, $2{cast})",
            column.table, column.primary_key, column.column
        );
        self.client
            .execute(sql.as_str(), &[&key, &value])
            .await
            .expect("the fixture row inserts");
    }

    async fn text_rows(&self, sql: &str, parameters: &[&(dyn ToSql + Sync)]) -> Vec<String> {
        self.client
            .query(sql, parameters)
            .await
            .expect("the fixture query runs")
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect()
    }

    async fn single_value(&self, sql: &str, key: &str) -> Option<Option<String>> {
        let row = self
            .client
            .query_opt(sql, &[&key])
            .await
            .expect("the fixture query runs");
        row.map(|record| record.get::<_, Option<String>>(0))
    }

    pub(crate) async fn stored(&self, column: &SecretColumn, key: &str) -> Option<Option<String>> {
        let sql = format!(
            "SELECT \"{}\"::text FROM \"{}\" WHERE \"{}\" = $1",
            column.column, column.table, column.primary_key
        );
        self.single_value(&sql, key).await
    }

    pub(crate) async fn snapshot(&self, column: &SecretColumn) -> Vec<String> {
        let sql = format!(
            "SELECT row_to_json(rows)::text FROM \"{}\" rows ORDER BY row_to_json(rows)::text",
            column.table
        );
        self.text_rows(&sql, &[]).await
    }

    pub(crate) async fn snapshot_except(&self, column: &SecretColumn, key: &str) -> Vec<String> {
        let sql = format!(
            "SELECT row_to_json(rows)::text FROM \"{}\" rows WHERE \"{}\" <> $1 ORDER BY row_to_json(rows)::text",
            column.table, column.primary_key
        );
        self.text_rows(&sql, &[&key]).await
    }

    pub(crate) async fn version(&self, column: &SecretColumn, key: &str) -> String {
        let sql = format!(
            "SELECT xmin::text FROM \"{}\" WHERE \"{}\" = $1",
            column.table, column.primary_key
        );
        self.single_value(&sql, key)
            .await
            .expect("the row exists")
            .expect("xmin is set")
    }
}

pub(crate) fn previous() -> SecretString {
    SecretString::from(PREVIOUS_KEY)
}

pub(crate) fn master() -> SecretString {
    SecretString::from(MASTER_KEY)
}

pub(crate) async fn migrate(
    database: &Database,
    previous: &SecretString,
    master: &SecretString,
    cipher: &DbCipher,
) -> Result<Option<MigrationOutcome>, MigrationFailure> {
    let request = MigrationRequest {
        previous_master_key: Some(previous),
        master_key: Some(master),
        salt_key_is_set: false,
        cipher,
        max_depth: 100,
    };
    migrate_if_requested(&request, || Ok(Some(&database.client)), Ok, Ok).await
}

pub(crate) fn stored_value(text: &str, is_json: bool) -> MigrationJson {
    if is_json {
        json(text)
    } else {
        MigrationJson::String(SecretText(SecretString::from(text)))
    }
}

pub(crate) fn plaintexts(text: &str, is_json: bool, key: &str) -> Vec<String> {
    decrypted_leaves(&stored_value(text, is_json), key)
}
