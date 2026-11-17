#![cfg(test)]

use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use clap::Parser as _;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing_subscriber::fmt::MakeWriter;

use betterlite_postgres_fixture::connection::PostgresFixture;

use crate::key_repository::table::columns::TokenColumn;
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::key_user_writes::WritePorts;
use crate::key_user_writes::columns::{UserColumn, UserWrite};
use crate::key_user_writes::retry::{FailureEvent, FailureFuture, FailureSink, WriteClock};
use crate::migration_ledger::lock::JitterSource;
use crate::tool_spend::RetryPause;

pub(crate) const BASELINE: &str =
    include_str!("../../../../../tools/postgres-fixture/fixtures/baseline.sql");

// token columns the baseline schema lacks, and the permission table a key update includes
pub(crate) const SCHEMA: &str = r#"
ALTER TABLE "LiteLLM_VerificationToken"
    ADD COLUMN IF NOT EXISTS object_permission_id TEXT,
    ADD COLUMN IF NOT EXISTS settings_updated_at TIMESTAMP(3),
    ADD COLUMN IF NOT EXISTS budget_limits JSONB;
CREATE TABLE IF NOT EXISTS "LiteLLM_ObjectPermissionTable" (
    "object_permission_id" TEXT NOT NULL,
    "mcp_servers" TEXT[] DEFAULT ARRAY[]::TEXT[],
    CONSTRAINT "LiteLLM_ObjectPermissionTable_pkey" PRIMARY KEY ("object_permission_id")
);
"#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Harness {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
}

impl Harness {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["key-user-writes-tests"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .unwrap();
        let client = pool.get().await.unwrap();
        client.batch_execute(BASELINE).await.unwrap();
        client.batch_execute(SCHEMA).await.unwrap();
        drop(client);
        Self { fixture, pool }
    }

    pub(crate) async fn client(&self) -> Client {
        self.pool.get().await.unwrap()
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    DateTime::from_timestamp(0, 0).unwrap()
}

pub(crate) struct StepClock {
    base: Instant,
    step: Duration,
    calls: Mutex<u32>,
}

impl StepClock {
    pub(crate) fn new(step: Duration) -> Self {
        Self {
            base: Instant::now(),
            step,
            calls: Mutex::new(0),
        }
    }
}

impl WriteClock for StepClock {
    fn now(&self) -> DateTime<Utc> {
        fixed_now()
    }

    fn instant(&self) -> Instant {
        let mut calls = lock(&self.calls);
        let index = *calls;
        *calls = calls.saturating_add(1);
        self.base
            .checked_add(self.step.saturating_mul(index))
            .unwrap_or(self.base)
    }
}

pub(crate) struct HalfJitter;

impl JitterSource for HalfJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        upper_exclusive_nanos.checked_shr(1).unwrap_or(0)
    }
}

// one log of a write's calls, so a test sees each pause after the failure event before it
pub(crate) type Timeline = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
pub(crate) struct RecordingPause {
    pub(crate) delays: Mutex<Vec<Duration>>,
    pub(crate) timeline: Timeline,
}

impl RecordingPause {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        lock(&self.delays).clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        lock(&self.delays).push(delay);
        let millis = delay.as_millis();
        lock(&self.timeline).push(format!("pause {millis}"));
        Box::pin(async {})
    }
}

#[derive(Default)]
pub(crate) struct RecordingSink {
    pub(crate) events: Mutex<Vec<FailureEvent>>,
    pub(crate) timeline: Timeline,
}

impl RecordingSink {
    pub(crate) fn events(&self) -> Vec<FailureEvent> {
        let mut events = lock(&self.events);
        std::mem::take(&mut *events)
    }
}

impl FailureSink for RecordingSink {
    fn record(&self, event: FailureEvent) -> FailureFuture<'_> {
        let label = event.call_type.as_str();
        lock(&self.timeline).push(format!("event {label}"));
        lock(&self.events).push(event);
        Box::pin(async {})
    }
}

pub(crate) struct Ports {
    pub(crate) clock: StepClock,
    pub(crate) jitter: HalfJitter,
    pub(crate) pause: RecordingPause,
    pub(crate) sink: RecordingSink,
}

impl Ports {
    pub(crate) fn new(step: Duration) -> Self {
        let timeline = Timeline::default();
        Self {
            clock: StepClock::new(step),
            jitter: HalfJitter,
            pause: RecordingPause {
                delays: Mutex::default(),
                timeline: Arc::clone(&timeline),
            },
            sink: RecordingSink {
                events: Mutex::default(),
                timeline,
            },
        }
    }

    pub(crate) fn timeline(&self) -> Vec<String> {
        lock(&self.pause.timeline).clone()
    }

    pub(crate) fn write_ports(&self) -> WritePorts<'_> {
        WritePorts {
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            sink: &self.sink,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

pub(crate) fn key_write(token: &str, alias: &str) -> TokenWrite {
    let mut fields = TokenWrite::default();
    fields
        .0
        .insert(TokenColumn::Token, TokenValue::Text(token.to_owned()));
    fields
        .0
        .insert(TokenColumn::KeyAlias, TokenValue::Text(alias.to_owned()));
    fields
}

pub(crate) fn key_column(token: &str, column: TokenColumn, value: TokenValue) -> TokenWrite {
    let mut fields = key_write(token, "unused");
    fields.0.insert(column, value);
    fields
}

pub(crate) fn failing_key_write() -> TokenWrite {
    key_column(
        "sk-retry",
        TokenColumn::MaxBudget,
        TokenValue::Text(String::from("not-a-number")),
    )
}

pub(crate) fn alias_update(alias: &str) -> TokenWrite {
    let mut fields = TokenWrite::default();
    fields
        .0
        .insert(TokenColumn::KeyAlias, TokenValue::Text(alias.to_owned()));
    fields
}

pub(crate) fn user_write(user_id: &str, alias: &str) -> UserWrite {
    let mut fields = UserWrite::default();
    fields
        .0
        .insert(UserColumn::UserId, TokenValue::Text(user_id.to_owned()));
    fields
        .0
        .insert(UserColumn::UserAlias, TokenValue::Text(alias.to_owned()));
    fields
}

pub(crate) fn user_column(user_id: &str, column: UserColumn, value: TokenValue) -> UserWrite {
    let mut fields = user_write(user_id, "unused");
    fields.0.insert(column, value);
    fields
}

pub(crate) fn user_alias_update(alias: &str) -> UserWrite {
    let mut fields = UserWrite::default();
    fields
        .0
        .insert(UserColumn::UserAlias, TokenValue::Text(alias.to_owned()));
    fields
}
