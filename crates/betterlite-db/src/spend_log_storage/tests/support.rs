use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, Config, NoTls};

use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;
use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::spend_log_storage::rows::writer::{SpendUpdateClient, SpendUpdateRequest};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError};
use crate::tool_spend::RetryPause;

const ROWS_SQL: &str = r#"SELECT request_id, call_type, status, spend::numeric FROM "LiteLLM_SpendLogs" ORDER BY request_id"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct SpendLogCase {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    pub(crate) client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl SpendLogCase {
    pub(crate) async fn new() -> Self {
        let url = Environment::parse_from(["spend-log-storage"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .expect("a test database");
        let store = pooled_store(&fixture);
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        Self {
            fixture,
            store,
            client,
            tasks,
        }
    }

    pub(crate) async fn close(self) {
        let Self {
            fixture,
            store,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        if let Some(pool) = store.pool {
            pool.close();
        }
        fixture
            .close_async()
            .await
            .expect("the database is dropped");
    }

    pub(crate) async fn rows(&self) -> Vec<(String, String, String, Decimal)> {
        let rows = self
            .client
            .query(ROWS_SQL, &[])
            .await
            .expect("a fixture query");
        rows.iter()
            .map(|row| {
                (
                    row.get("request_id"),
                    row.get("call_type"),
                    row.get("status"),
                    row.get("spend"),
                )
            })
            .collect()
    }

    pub(crate) async fn execute(&self, sql: &str) {
        self.client
            .batch_execute(sql)
            .await
            .expect("a fixture statement");
    }
}

fn pooled_store(fixture: &PostgresFixture) -> PostgresStore {
    let manager = Manager::new(fixture.options.config.clone(), NoTls);
    let pool = Pool::builder(manager)
        .max_size(2)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}

pub(crate) fn unreachable_store() -> PostgresStore {
    let configuration: Config = "postgres://spend@127.0.0.1:1/spend"
        .parse()
        .expect("a URL no server listens on");
    let manager = Manager::new(configuration, NoTls);
    let pool = Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("a connection pool");
    PostgresStore { pool: Some(pool) }
}

pub(crate) fn text(value: &str) -> SpendLogValue {
    SpendLogValue::Text(value.to_owned())
}

pub(crate) const fn spend(amount: Decimal) -> SpendLogValue {
    SpendLogValue::Decimal(amount)
}

fn row(request_id: &str, call_type: &str, status: &str, amount: Decimal) -> SpendLogRow {
    BTreeMap::from([
        ("request_id".to_owned(), text(request_id)),
        ("call_type".to_owned(), text(call_type)),
        ("status".to_owned(), text(status)),
        ("spend".to_owned(), spend(amount)),
        ("startTime".to_owned(), text("2026-01-02T03:04:05.000")),
        ("endTime".to_owned(), text("2026-01-02T03:04:06.000")),
        ("model".to_owned(), text("gpt-4o")),
        ("total_tokens".to_owned(), SpendLogValue::Whole(10)),
    ])
}

pub(crate) fn spend_row(request_id: &str) -> SpendLogRow {
    row(request_id, "acompletion", "success", Decimal::new(25, 2))
}

pub(crate) fn batch_cost_row(request_id: &str, status: &str, amount: Decimal) -> SpendLogRow {
    row(request_id, "aretrieve_batch", status, amount)
}

pub(crate) fn ordinary_row(request_id: &str) -> SpendLogRow {
    row(request_id, "acompletion", "success", Decimal::new(25, 2))
}

pub(crate) fn text_of(entry: &SpendLogRow, field: &str) -> Option<String> {
    match entry.get(field) {
        Some(SpendLogValue::Text(value)) => Some(value.clone()),
        Some(_) | None => None,
    }
}

pub(crate) fn decimal_of(entry: &SpendLogRow, field: &str) -> Option<Decimal> {
    match entry.get(field) {
        Some(SpendLogValue::Decimal(amount)) => Some(*amount),
        Some(SpendLogValue::Whole(amount)) => Some(Decimal::from(*amount)),
        Some(_) | None => None,
    }
}

pub(crate) fn ids(rows: &[SpendLogRow]) -> Vec<String> {
    rows.iter()
        .map(|entry| text_of(entry, "request_id").unwrap_or_default())
        .collect()
}

// A scripted prisma error, each flag set the way a native or external failure would report itself.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend log insert failed")]
pub(crate) struct ScriptedFailure {
    data_rejection: bool,
    service_unavailable: bool,
    deadlock: bool,
    database: bool,
}

impl ScriptedFailure {
    pub(crate) const fn data() -> Self {
        Self {
            data_rejection: true,
            service_unavailable: false,
            deadlock: false,
            database: true,
        }
    }

    pub(crate) const fn outage() -> Self {
        Self {
            data_rejection: false,
            service_unavailable: true,
            deadlock: false,
            database: true,
        }
    }

    pub(crate) const fn mislabelled_outage() -> Self {
        Self {
            data_rejection: true,
            service_unavailable: true,
            deadlock: false,
            database: true,
        }
    }

    pub(crate) const fn mislabelled_deadlock() -> Self {
        Self {
            data_rejection: true,
            service_unavailable: false,
            deadlock: true,
            database: true,
        }
    }

    pub(crate) const fn database_error() -> Self {
        Self {
            data_rejection: false,
            service_unavailable: false,
            deadlock: false,
            database: true,
        }
    }

    pub(crate) const fn non_database() -> Self {
        Self {
            data_rejection: false,
            service_unavailable: false,
            deadlock: false,
            database: false,
        }
    }
}

impl SpendLogWriteError for ScriptedFailure {
    fn is_transient(&self) -> bool {
        self.service_unavailable || self.deadlock
    }

    fn is_data_rejection(&self) -> bool {
        self.data_rejection
    }

    fn is_service_unavailable(&self) -> bool {
        self.service_unavailable
    }

    fn is_deadlock(&self) -> bool {
        self.deadlock
    }

    fn is_database(&self) -> bool {
        self.database
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the external spend writer is unreachable")]
pub(crate) struct NoHttpFailure;

// The external writer a store can be attached to when only the database branch should run.
pub(crate) struct NoClient;

impl SpendUpdateClient for NoClient {
    type Error = NoHttpFailure;

    fn post<'call>(
        &'call self,
        _request: SpendUpdateRequest<'call>,
    ) -> SpendLogFuture<'call, Result<u16, NoHttpFailure>> {
        Box::pin(async { Err(NoHttpFailure) })
    }
}

// RetryPause that records the wait instead of taking it.
#[derive(Default)]
pub(crate) struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(ready(()))
    }
}

// WriteClock whose second instant is five seconds after its first, so a reported duration is exact.
pub(crate) struct SteppingClock {
    pub(crate) start: Instant,
    pub(crate) steps: AtomicU32,
}

impl WriteClock for SteppingClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn instant(&self) -> Instant {
        let step = self.steps.fetch_add(1, Ordering::SeqCst);
        self.start + Duration::from_secs(u64::from(step).saturating_mul(5))
    }
}
