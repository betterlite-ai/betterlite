#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::cloudzero::UsageRow;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_logging::finops::cloudzero::delivery::payload::ApiRecord;
use betterlite_logging::finops::cloudzero::delivery::transport::{
    BillingDropRequest, BillingTransport, Clock,
};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone as _, Utc};
use deadpool_postgres::{Manager, Pool};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Number;
use tokio_postgres::types::ToSql;

use crate::finops::cloudzero::schedule::background::{CloudZeroExportJob, CloudZeroScheduler};
use crate::finops::cloudzero::schedule::lease::UsageExportLease;
use crate::finops::cloudzero::schedule::{CloudZeroLogger, CloudZeroSettings, ExportPipeline};

pub(crate) const API_KEY: &str = "cz-api-key-1234";
pub(crate) const CONNECTION_ID: &str = "conn-5678";

pub(crate) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn settings() -> CloudZeroSettings {
    let empty: BTreeMap<String, String> = BTreeMap::new();
    CloudZeroSettings::resolve(Some(API_KEY), Some(CONNECTION_ID), None, &empty)
        .expect("default settings")
}

pub(crate) fn settings_from(pairs: &[(&str, &str)]) -> CloudZeroSettings {
    CloudZeroSettings::resolve(
        Some(API_KEY),
        Some(CONNECTION_ID),
        None,
        &environment(pairs),
    )
    .expect("settings from the environment")
}

pub(crate) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 2, 3, 4, 5)
        .single()
        .expect("a valid instant")
}

pub(crate) fn minutes_ago(offset_minutes: i64) -> NaiveDateTime {
    now()
        .naive_utc()
        .checked_sub_signed(TimeDelta::try_minutes(offset_minutes).expect("a duration"))
        .expect("an earlier instant")
}

pub(crate) fn datastore() -> PostgresStore {
    PostgresStore { pool: None }
}

pub(crate) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(crate) fn integer(value: i64) -> JsonData {
    JsonData::Number(Number::from(value))
}

pub(crate) fn decimal(value: &str) -> JsonData {
    JsonData::Number(serde_json::from_str(value).expect("a JSON number"))
}

pub(crate) fn row(fields: &[(&str, JsonData)]) -> UsageRow {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[derive(Debug, thiserror::Error)]
#[error("the fake transport refused the post")]
pub(crate) struct Refused;

#[derive(Clone, Debug)]
pub(crate) struct Posted {
    pub month: String,
    pub operation: String,
    pub data: Vec<ApiRecord>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FakeTransport {
    posted: Arc<Mutex<Vec<Posted>>>,
    fail: bool,
}

impl FakeTransport {
    pub(crate) fn failing() -> Self {
        Self {
            posted: Arc::new(Mutex::new(Vec::new())),
            fail: true,
        }
    }

    pub(crate) fn posts(&self) -> Vec<Posted> {
        self.posted.lock().expect("an unsplit lock").clone()
    }
}

impl BillingTransport for FakeTransport {
    type Error = Refused;

    fn post<'request>(
        &'request self,
        request: &'request BillingDropRequest<'request>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'request {
        self.posted.lock().expect("an unsplit lock").push(Posted {
            month: request.body.month.clone(),
            operation: request.body.operation.to_owned(),
            data: request.body.data.clone(),
        });
        ready(if self.fail { Err(Refused) } else { Ok(()) })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FakeLease {
    pub(crate) redis: bool,
    pub(crate) acquire_result: Option<bool>,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
}

impl FakeLease {
    pub(crate) fn new(redis: bool, acquire_result: Option<bool>) -> Self {
        Self {
            redis,
            acquire_result,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events.lock().expect("an unsplit lock").clone()
    }
}

#[async_trait]
impl UsageExportLease for FakeLease {
    fn redis_available(&self) -> bool {
        self.redis
    }

    async fn acquire_lock(&self, cronjob_id: &str) -> Option<bool> {
        self.events
            .lock()
            .expect("an unsplit lock")
            .push(format!("acquire:{cronjob_id}"));
        self.acquire_result
    }

    async fn release_lock(&self, cronjob_id: &str) {
        self.events
            .lock()
            .expect("an unsplit lock")
            .push(format!("release:{cronjob_id}"));
    }
}

#[derive(Default)]
pub(crate) struct FakeScheduler {
    pub(crate) jobs: Vec<(String, i64, Arc<dyn CloudZeroExportJob>)>,
}

impl CloudZeroScheduler for FakeScheduler {
    fn add_interval_job(
        &mut self,
        job_id: &str,
        interval_minutes: i64,
        job: Arc<dyn CloudZeroExportJob>,
    ) {
        self.jobs.push((job_id.to_owned(), interval_minutes, job));
    }
}

pub(crate) fn logger(
    settings: CloudZeroSettings,
    store: PostgresStore,
    transport: FakeTransport,
    lease: Option<Arc<dyn UsageExportLease>>,
) -> CloudZeroLogger<FakeTransport, FixedClock> {
    CloudZeroLogger {
        settings,
        pipeline: ExportPipeline {
            store,
            timezone_database: TimeZoneDatabase::from_env(),
            clock: FixedClock(now()),
            transport,
        },
        lease,
    }
}

pub(crate) fn leased(lease: &Arc<FakeLease>) -> Option<Arc<dyn UsageExportLease>> {
    let held: Arc<dyn UsageExportLease> = Arc::clone(lease);
    Some(held)
}

pub(crate) fn lease_events(job: &str) -> Vec<String> {
    vec![format!("acquire:{job}"), format!("release:{job}")]
}

const USAGE_INSERT: &str = r#"INSERT INTO "LiteLLM_DailyUserSpend"
    (id, user_id, date, api_key, model, model_group, custom_llm_provider,
     prompt_tokens, completion_tokens, spend, api_requests, successful_requests,
     failed_requests, created_at, updated_at)
    VALUES ($1, 'user-1', $2, $3, $4, $5, 'openai', $6, $7, $8::numeric, 1, $9, 0, $10, $10)"#;

const GENERATED_USAGE_INSERT: &str = r#"INSERT INTO "LiteLLM_DailyUserSpend"
    (id, user_id, date, api_key, model, model_group, custom_llm_provider,
     prompt_tokens, completion_tokens, spend, api_requests, successful_requests,
     failed_requests, created_at, updated_at)
    SELECT 'row-' || g, 'user-1', '2026-01-01', 'sk-generated', 'model-' || g, 'group-1',
           'openai', 2, 1, 1.5, 1, 1, 0, $1, $1
    FROM generate_series(1::bigint, $2::bigint) AS g"#;

pub(crate) struct UsageSpec<'row> {
    pub id: &'row str,
    pub date: &'row str,
    pub api_key: &'row str,
    pub model: &'row str,
    pub model_group: Option<&'row str>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub spend: Decimal,
    pub successful_requests: i64,
    pub updated_at: NaiveDateTime,
}

impl<'row> UsageSpec<'row> {
    fn values(&self) -> [&(dyn ToSql + Sync); 10] {
        [
            &self.id,
            &self.date,
            &self.api_key,
            &self.model,
            &self.model_group,
            &self.prompt_tokens,
            &self.completion_tokens,
            &self.spend,
            &self.successful_requests,
            &self.updated_at,
        ]
    }
}

pub(crate) fn usage<'row>(
    id: &'row str,
    model: &'row str,
    spend: Decimal,
    updated_at: NaiveDateTime,
) -> UsageSpec<'row> {
    UsageSpec {
        id,
        date: "2026-01-01",
        api_key: "sk-usage-key",
        model,
        model_group: Some("group-1"),
        prompt_tokens: 2,
        completion_tokens: 1,
        spend,
        successful_requests: 1,
        updated_at,
    }
}

pub(crate) async fn insert_usage(store: &PostgresStore, spec: &UsageSpec<'_>) {
    let client = store
        .pool
        .as_ref()
        .expect("a pool")
        .get()
        .await
        .expect("a client");
    client
        .execute(USAGE_INSERT, &spec.values())
        .await
        .expect("a usage row");
}

pub(crate) async fn insert_usage_specs(store: &PostgresStore, specs: &[UsageSpec<'_>]) {
    for spec in specs {
        insert_usage(store, spec).await;
    }
}

pub(crate) async fn insert_generated_usage(store: &PostgresStore, at: NaiveDateTime, rows: i64) {
    let client = store
        .pool
        .as_ref()
        .expect("a pool")
        .get()
        .await
        .expect("a client");
    client
        .execute(GENERATED_USAGE_INSERT, &[&at, &rows])
        .await
        .expect("generated usage rows");
}

pub(crate) async fn database() -> (PostgresFixture, PostgresStore) {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct Environment {
        #[arg(long, env = "POSTGRES_TEST_URL")]
        url: Option<String>,
    }

    let url = Environment::parse_from(["cloudzero-schedule-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(
        url.as_ref(),
        catalog_statements(&PACKAGED_CATALOGS).expect("catalog statements"),
    )
    .resolve()
    .await
    .expect("a provisioned database");
    let mut config = fixture.options.config.clone();
    let current = config.get_options().unwrap_or_default().to_owned();
    config.options(format!("{current} -ctimezone=UTC"));
    let manager = Manager::new(config, tokio_postgres::NoTls);
    let pool = Pool::builder(manager).max_size(3).build().expect("a pool");
    (fixture, PostgresStore { pool: Some(pool) })
}

pub(crate) fn clone_store(store: &PostgresStore) -> PostgresStore {
    PostgresStore {
        pool: store.pool.clone(),
    }
}

pub(crate) async fn close(fixture: PostgresFixture, store: &PostgresStore) {
    if let Some(pool) = &store.pool {
        pool.close();
    }
    fixture
        .close_async()
        .await
        .expect("the database is dropped");
}
