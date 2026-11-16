#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_db::daily_activity_contracts::{
    DailyActivityProxyReads, KeyMetadataRow, SpendLogsWindow,
};
use betterlite_db::key_attribution::history::recovery::SpendLogHistory;
use betterlite_db::postgres_store::session::PostgresStore as Store;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
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
    pub(crate) store: Store,
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

pub(crate) fn reads(store: &Store) -> ProxyDailyActivityReads {
    ProxyDailyActivityReads {
        store: store.clone(),
        history: Arc::new(SpendLogHistory::default()),
    }
}

pub(crate) fn resolved_row(api_key: &str, user_id: &str) -> KeyMetadataRow {
    KeyMetadataRow {
        api_key: api_key.to_owned(),
        key_alias: None,
        team_id: None,
        user_id: Some(user_id.to_owned()),
        user_email: None,
        key_exists: false,
        tags: Vec::new(),
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
        store: Store { pool: Some(pool) },
    }
}

pub(crate) async fn close(database: Database) {
    if let Some(pool) = database.store.pool {
        pool.close();
    }
    database.fixture.close_async().await.unwrap();
}

pub(crate) async fn client(store: &Store) -> Client {
    store.pool.as_ref().unwrap().get().await.unwrap()
}

const INSERT_USER: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, teams) VALUES ($1, $2, $3)"#;
const INSERT_BROKEN_USER: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_email, metadata) VALUES ($1, $2, '"not-json"'::jsonb)"#;
const INSERT_TOKEN: &str =
    r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, user_id) VALUES ($1, $2, $3)"#;
const INSERT_TOMBSTONE: &str = r#"INSERT INTO "LiteLLM_DeletedVerificationToken" (id, token, key_alias, deleted_at) VALUES ($1, $2, $3, $4)"#;
const INSERT_SPEND_LOG: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime", "user", metadata) VALUES ($1, 'completion', $2, $3, $3, $4, $5::text::jsonb)"#;
const INSERT_DAILY_SPEND: &str = r#"INSERT INTO "LiteLLM_DailyUserSpend" (id, user_id, date, api_key, model, updated_at) VALUES ($1, $2, $3, $4, '', NOW())"#;

async fn insert_one(store: &Store, statement: &str, params: &[&(dyn ToSql + Sync)]) {
    let inserted = client(store)
        .await
        .execute(statement, params)
        .await
        .unwrap();
    assert_eq!(inserted, 1);
}

pub(crate) async fn insert_user(store: &Store, user_id: &str, email: Option<&str>, teams: &[&str]) {
    let members: Vec<String> = teams.iter().map(|team| (*team).to_owned()).collect();
    insert_one(store, INSERT_USER, &[&user_id, &email, &members]).await;
}

pub(crate) async fn insert_broken_user(store: &Store, user_id: &str) {
    let email = format!("{user_id}@example.com");
    insert_one(store, INSERT_BROKEN_USER, &[&user_id, &email]).await;
}

pub(crate) async fn insert_token(store: &Store, token: &str, alias: &str, user_id: Option<&str>) {
    insert_one(store, INSERT_TOKEN, &[&token, &alias, &user_id]).await;
}

pub(crate) async fn insert_tombstone(
    store: &Store,
    id: &str,
    token: &str,
    alias: &str,
    deleted_at: NaiveDateTime,
) {
    insert_one(store, INSERT_TOMBSTONE, &[&id, &token, &alias, &deleted_at]).await;
}

pub(crate) async fn insert_spend_log(
    store: &Store,
    api_key: &str,
    started: NaiveDateTime,
    alias: &str,
    user_id: Option<&str>,
) {
    let metadata = format!(r#"{{"user_api_key_alias":"{alias}"}}"#);
    let request_id = uuid::Uuid::new_v4().to_string();
    let params: [&(dyn ToSql + Sync); 5] = [&request_id, &api_key, &started, &user_id, &metadata];
    insert_one(store, INSERT_SPEND_LOG, &params).await;
}

pub(crate) async fn insert_daily_spend(store: &Store, api_key: &str, user_id: &str, date: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    insert_one(store, INSERT_DAILY_SPEND, &[&id, &user_id, &date, &api_key]).await;
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
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
