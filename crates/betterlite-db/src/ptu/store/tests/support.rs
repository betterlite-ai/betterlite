use std::future::{Future, ready};
use std::io::{Result as IoResult, Write};
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;

use crate::ptu::charges::pricing::PtuCharge;
use crate::ptu::store::sentinel::ChargeWrite;
use crate::ptu::store::sleeper::Sleeper;
use crate::schema_agent_identity::MIGRATION_CATALOG as AGENT_IDENTITY;
use crate::schema_baseline::MIGRATION_CATALOG as BASELINE;
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS;
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["ptu-store-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(
        fixture.options.url.expose_secret().parse().unwrap(),
        tokio_postgres::NoTls,
    );
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    let client = pool.get().await.unwrap();
    for catalog in [
        BASELINE,
        DAILY_USAGE,
        MANAGED_RESOURCES,
        IDENTITY_HISTORY,
        POLICY_RESOURCES,
        ROUTING_STATE,
        BUDGET_WINDOWS,
        AGENT_IDENTITY,
    ] {
        for migration in catalog.files {
            let sql = str::from_utf8(migration.bytes).unwrap();
            client.batch_execute(sql).await.unwrap();
        }
    }
    (fixture, pool)
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) delays: Vec<Duration>,
}

impl Sleeper for Recorder {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.delays.push(duration);
        ready(())
    }
}

pub(crate) fn charge(team: &str, id: &str, name: &str, cost: Decimal) -> PtuCharge {
    PtuCharge {
        team_id: team.to_owned(),
        model_id: id.to_owned(),
        model_name: name.to_owned(),
        flat_cost: cost,
    }
}

pub(crate) fn write_for<'charge>(charge: &'charge PtuCharge, date: &str) -> ChargeWrite<'charge> {
    ChargeWrite {
        charge,
        date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        id: uuid::Uuid::new_v4(),
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-03-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
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

pub(crate) fn capture_logs() -> (CapturedLogs, impl Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}
