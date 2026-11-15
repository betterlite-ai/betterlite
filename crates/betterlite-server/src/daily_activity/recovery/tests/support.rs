#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_db::daily_activity_contracts::{
    DailyActivityProxyReads, KeyMetadataRow, SpendLogsWindow,
};
use betterlite_db::key_attribution::history::recovery::SpendLogHistory;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio_postgres::NoTls;
use tracing_subscriber::fmt::MakeWriter;

use crate::daily_activity::recovery::reads::ProxyDailyActivityReads;

pub(crate) const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub(crate) const OTHER_DIGEST: &str =
    "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

const MIGRATIONS: &str = concat!(
    include_str!("../../../../../betterlite-db/migrations/20250326162113_baseline/migration.sql"),
    include_str!(
        "../../../../../betterlite-db/migrations/20250326171002_add_daily_user_table/migration.sql"
    ),
    include_str!(
        "../../../../../betterlite-db/migrations/20251219110931_add_deleted_keys_and_deleted_teams_tables/migration.sql"
    ),
);

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
}

pub(crate) fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
        .and_utc()
}

pub(crate) fn moment(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

pub(crate) fn window(start: DateTime<Utc>, end: DateTime<Utc>) -> SpendLogsWindow {
    (SystemTime::from(start), SystemTime::from(end))
}

pub(crate) fn dates(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn keys(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn digest_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) fn reads(store: &PostgresStore) -> ProxyDailyActivityReads {
    ProxyDailyActivityReads {
        store: store.clone(),
        history: Arc::new(SpendLogHistory::default()),
    }
}

pub(crate) async fn recover(
    reader: &ProxyDailyActivityReads,
    resolved: BTreeMap<String, KeyMetadataRow>,
    api_keys: &BTreeSet<String>,
    window: Option<SpendLogsWindow>,
) -> BTreeMap<String, KeyMetadataRow> {
    DailyActivityProxyReads::recover_key_metadata(reader, resolved, api_keys, window).await
}

pub(crate) async fn database() -> Database {
    let url = Environment::parse_from(["daily-activity-recovery-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(3).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(MIGRATIONS).await.unwrap();
    drop(client);
    Database {
        fixture,
        store: PostgresStore { pool: Some(pool) },
    }
}

pub(crate) async fn close(database: Database) {
    if let Some(pool) = database.store.pool {
        pool.close();
    }
    database.fixture.close_async().await.unwrap();
}

pub(crate) async fn client(store: &PostgresStore) -> Client {
    store.pool.as_ref().unwrap().get().await.unwrap()
}

pub(crate) async fn insert_user(
    store: &PostgresStore,
    user_id: &str,
    email: Option<&str>,
    teams: &[&str],
) {
    let teams: Vec<String> = teams.iter().map(|team| (*team).to_owned()).collect();
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, teams) VALUES ($1, $2, $3)"#,
            &[&user_id, &email, &teams],
        )
        .await
        .unwrap();
}

pub(crate) async fn insert_broken_user(store: &PostgresStore, user_id: &str) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, metadata) VALUES ($1, $2, '"not-json"'::jsonb)"#,
            &[&user_id, &format!("{user_id}@example.com")],
        )
        .await
        .unwrap();
}

pub(crate) async fn insert_token(
    store: &PostgresStore,
    token: &str,
    alias: &str,
    user_id: Option<&str>,
) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, user_id) VALUES ($1, $2, $3)"#,
            &[&token, &alias, &user_id],
        )
        .await
        .unwrap();
}

pub(crate) async fn insert_tombstone(
    store: &PostgresStore,
    id: &str,
    token: &str,
    alias: &str,
    deleted_at: NaiveDateTime,
) {
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, key_alias, deleted_at) VALUES ($1, $2, $3, $4)"#,
            &[&id, &token, &alias, &deleted_at],
        )
        .await
        .unwrap();
}

pub(crate) async fn insert_spend_log(
    store: &PostgresStore,
    api_key: &str,
    started: NaiveDateTime,
    alias: &str,
    user_id: Option<&str>,
) {
    let metadata = format!(r#"{{"user_api_key_alias":"{alias}"}}"#);
    let request_id = uuid::Uuid::new_v4().to_string();
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime", "user", metadata) VALUES ($1, 'completion', $2, $3, $3, $4, $5::text::jsonb)"#,
            &[&request_id, &api_key, &started, &user_id, &metadata],
        )
        .await
        .unwrap();
}

pub(crate) async fn insert_daily_spend(
    store: &PostgresStore,
    api_key: &str,
    user_id: &str,
    date: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    client(store)
        .await
        .execute(
            r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, date, api_key, model, updated_at) VALUES ($1, $2, $3, $4, '', NOW())"#,
            &[&id, &user_id, &date, &api_key],
        )
        .await
        .unwrap();
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (
    CapturedLogs,
    impl tracing::Subscriber + Send + Sync + 'static,
) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}
