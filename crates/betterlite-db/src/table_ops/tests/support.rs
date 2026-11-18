#![cfg(test)]

use crate::key_repository::table::TokenValue;
use crate::table_ops::columns::TeamColumn;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls};

use betterlite_postgres_fixture::connection::PostgresFixture;

use crate::budget_permission_rows::{JsonData, Timestamp};
use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::pinned::WriterPinnedClient;
use crate::key_user_writes::retry::{SystemWriteClock, WriteClock};
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::migration_ledger::lock::JitterSource;
use crate::table_ops::{
    ConfigParamInvalidator, FailureFuture, FailureSink, InvalidateFuture, OperationCallType,
    TableFailureEvent, TableOps, TablePorts, TableSettings,
};
use crate::tool_spend::TokioPause;

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
        let configured = Environment::parse_from(["table_ops-tests"])
            .url
            .map(SecretString::from);
        let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the catalogs are UTF-8");
        let fixture = PostgresFixture::provision_with_statements(configured.as_ref(), statements)
            .resolve()
            .await
            .expect("a Postgres database fixture");
        let config = fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .expect("the fixture pool builds");
        Self { fixture, pool }
    }

    // a second pool over the same database, so a test can drain it and watch a call wait for a connection
    pub(crate) fn pool_with_size(&self, size: usize) -> Pool {
        let config = self
            .fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        Pool::builder(Manager::new(config, NoTls))
            .max_size(size)
            .build()
            .expect("the sized pool builds")
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture database drops");
    }
}

// the route B714 serves every table through hands out the same pool for each LiteLLM table
pub(crate) struct TableClient {
    pool: Pool,
}

impl TableClient {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

impl DatabaseLifecycle for TableClient {
    type Error = Infallible;

    fn is_connected(&self) -> bool {
        true
    }

    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(false) })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for TableClient {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        let known = [
            "litellm_usertable",
            "litellm_verificationtoken",
            "litellm_config",
            "litellm_spendlogs",
            "litellm_budgettable",
            "litellm_endusertable",
            "litellm_teamtable",
            "litellm_usernotifications",
            "query_raw",
        ];
        Ok(known
            .contains(&name)
            .then(|| Attribute::Model(self.pool.clone())))
    }

    fn token_refresh_enabled(&self) -> bool {
        false
    }

    fn token_label(&self) -> &'static str {
        "database token"
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}

// a zero jitter makes backoff's pause zero-length, so a retried failure costs no wall time
pub(crate) struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedFailure {
    pub(crate) call_type: OperationCallType,
    pub(crate) traceback: String,
    pub(crate) message: String,
}

#[derive(Default)]
pub(crate) struct RecordingSink {
    events: Mutex<Vec<RecordedFailure>>,
}

impl RecordingSink {
    pub(crate) fn events(&self) -> Vec<RecordedFailure> {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *events)
    }
}

impl FailureSink<Infallible> for RecordingSink {
    fn record(&self, event: TableFailureEvent<'_, Infallible>) -> FailureFuture<'_> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedFailure {
                call_type: event.call_type,
                traceback: event.traceback,
                message: event.error.to_string(),
            });
        Box::pin(async {})
    }
}

#[derive(Default)]
pub(crate) struct RecordingInvalidator {
    params: Mutex<Vec<String>>,
}

impl RecordingInvalidator {
    pub(crate) fn params(&self) -> Vec<String> {
        let mut params = self.params.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *params)
    }
}

impl ConfigParamInvalidator for RecordingInvalidator {
    fn invalidate_param<'call>(&'call self, param_name: &'call str) -> InvalidateFuture<'call> {
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(param_name.to_owned());
        Box::pin(async {})
    }
}

// a clock the test sets, so the timestamp Prisma's @updatedAt writes is the test's own value
pub(crate) struct ManualClock {
    now: DateTime<Utc>,
}

impl ManualClock {
    pub(crate) fn new(seconds: i64) -> Self {
        let now = DateTime::from_timestamp(seconds, 0).expect("the timestamp is in range");
        Self { now }
    }
}

impl WriteClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

pub(crate) struct Ports {
    pub(crate) clock: Box<dyn WriteClock>,
    pub(crate) jitter: ZeroJitter,
    pub(crate) pause: TokioPause,
    pub(crate) sink: RecordingSink,
    pub(crate) invalidator: RecordingInvalidator,
}

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the JSON parses")
}

// one insert helper keeps every fixture row's SQL beside the test that reads it
pub(crate) async fn seed(db: &Client, sql: &str, parameters: &[&(dyn ToSql + Sync)]) {
    db.execute(sql, parameters)
        .await
        .expect("the fixture row commits");
}

pub(crate) fn at(seconds: i64) -> Timestamp {
    let moment = chrono::DateTime::from_timestamp(seconds, 0).expect("the timestamp is in range");
    Timestamp::Naive(moment.naive_utc())
}

impl Ports {
    pub(crate) fn new() -> Self {
        Self::with_clock(Box::new(SystemWriteClock))
    }

    pub(crate) fn with_clock(clock: Box<dyn WriteClock>) -> Self {
        Self {
            clock,
            jitter: ZeroJitter,
            pause: TokioPause,
            sink: RecordingSink::default(),
            invalidator: RecordingInvalidator::default(),
        }
    }

    pub(crate) fn operations<'ports>(
        &'ports self,
        client: &'ports TableClient,
    ) -> TableOps<'ports, TableClient> {
        self.operations_with(client, TableSettings::default())
    }

    pub(crate) fn operations_with<'ports>(
        &'ports self,
        client: &'ports TableClient,
        settings: TableSettings,
    ) -> TableOps<'ports, TableClient> {
        TableOps {
            database: Some(WriterPinnedClient::Writer(client)),
            ports: self.table_ports(),
            settings,
        }
    }

    pub(crate) fn offline(&self) -> TableOps<'_, TableClient> {
        TableOps {
            database: None,
            ports: self.table_ports(),
            settings: TableSettings::default(),
        }
    }

    fn table_ports(&self) -> TablePorts<'_, Infallible> {
        TablePorts {
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            sink: &self.sink,
            invalidator: &self.invalidator,
        }
    }
}

pub(crate) fn team_payload(team_id: &str, alias: &str) -> BTreeMap<TeamColumn, TokenValue> {
    BTreeMap::from([
        (TeamColumn::TeamId, TokenValue::Text(team_id.to_owned())),
        (TeamColumn::TeamAlias, TokenValue::Text(alias.to_owned())),
        (TeamColumn::Admins, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Members, TokenValue::TextArray(Vec::new())),
        (TeamColumn::Models, TokenValue::TextArray(Vec::new())),
    ])
}

pub(crate) async fn text_column(db: &Client, sql: &str, parameter: &str) -> Option<String> {
    db.query_opt(sql, &[&parameter])
        .await
        .expect("the column query runs")
        .map(|row| row.get(0))
}
