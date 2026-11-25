use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;
use std::future::ready;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::clock::SystemClock as UptimeClock;
use betterlite_auth::reservation_settlement::adjust::AdjustmentMode;
use betterlite_auth::reservation_settlement::backend::SpendCounterBackend;
use betterlite_auth::reservation_settlement::entry::ReservationEntry;
use betterlite_auth::reservation_settlement::error::SettlementError;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_auth::reservation_settlement::reconcile::{
    ReservationFinality, reconcile_budget_reservation,
};
use betterlite_auth::reservation_settlement::release::{
    invalidate_reservation_keys, release_budget_reservation,
};
use betterlite_auth::spend_counter_batch::scope::SpendCounterScope;
use betterlite_auth::spend_increments::SpendIncrementServices;
use betterlite_auth::spend_increments::window::{WindowQueueFuture, WindowSpendQueue};
use betterlite_auth::spend_reseed::locks::LockRegistry;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_auth::spend_reseed::sql::SqlSpendCounterRows;
use betterlite_auth::spend_writes::SpendCounterWrites;
use betterlite_auth::spend_writes::batch::CounterBatch;
use betterlite_auth::spend_writes::source::SourceSpendCache;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::spend_log_storage::batch_claim::SpendLogAttachment;
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_log_storage::rows::SpendLogQueue;
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::spend_writer::accounting::{
    AdmissionFuture, SpendAccounting, SpendAccountingAdmission,
};
use betterlite_db::spend_writer::request::{PayloadFuture, SpendPayloadSource, SpendUpdateRequest};
use betterlite_db::spend_writer::settings::SpendWriterSettings;
use betterlite_db::spend_writer::{
    DbSpendUpdateWriter, NoSpendWriterFailureSink, SpendWriterQueues,
};
use betterlite_db::window_queue::WindowSpendTransaction;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis::request_batch::lock;
use betterlite_redis_fixture::server::RedisFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use redis::aio::MultiplexedConnection;
use redis::{Client, cmd};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use super::super::ports::{DatabaseSpendWrite, RequestCounterBatch, ReservationSettlement};
use super::super::{SettlementInput, SettlementPorts, settle_spend_write};
use super::journal::Recorder;
use crate::savings::Context;

const COUNTER: &str = "spend:user:u1";
const REQUEST_ID: &str = "batch-b1719";
const BATCH_ROW: &str = r#"{"request_id":"batch-b1719","call_type":"aretrieve_batch","status":"success","startTime":"2026-01-02T03:04:05.000","endTime":"2026-01-02T03:04:06.000","model":"gpt-4o"}"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0_u64))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

struct NoSources;

#[async_trait]
impl SourceSpendCache for NoSources {
    async fn cached(&self, _cache_key: &str) -> Option<Json> {
        None
    }
}

struct NoWindows;

impl WindowSpendQueue for NoWindows {
    fn enqueue(&self, _transaction: WindowSpendTransaction) -> WindowQueueFuture<'_> {
        Box::pin(ready(Ok(())))
    }
}

struct Admitted;

impl SpendAccountingAdmission for Admitted {
    fn admit(&self, _work: SpendAccounting) -> AdmissionFuture<'_> {
        Box::pin(ready(Ok(())))
    }
}

struct BatchRow;

impl SpendPayloadSource for BatchRow {
    type Error = Infallible;

    fn build(&self) -> PayloadFuture<'_, Infallible> {
        Box::pin(ready(Ok(BATCH_ROW.as_bytes().to_vec())))
    }
}

// the settlement port as B8481 maps it, onto B3003 over the same counters the increment writes
struct CounterSettlement<'backend> {
    backend: SpendCounterBackend<'backend, DualCache<LocalMemoryOps>, SqlSpendCounterRows>,
}

#[async_trait]
impl ReservationSettlement for CounterSettlement<'_> {
    async fn reconcile(
        &self,
        reservation: &ReservationHandle,
        actual_cost: Decimal,
        finality: ReservationFinality,
        mode: AdjustmentMode,
    ) -> Result<(), SettlementError> {
        let _pending =
            reconcile_budget_reservation(&self.backend, reservation, actual_cost, finality, mode)
                .await?;
        Ok(())
    }

    async fn release(&self, reservation: &ReservationHandle) -> Result<(), SettlementError> {
        release_budget_reservation(&self.backend, reservation).await
    }

    async fn invalidate(&self, reservation: &ReservationHandle) -> Result<(), SettlementError> {
        invalidate_reservation_keys(&self.backend, reservation).await;
        Ok(())
    }
}

async fn redis_cache(fixture: &RedisFixture) -> Arc<RedisCache> {
    let url = serde_json::to_string(fixture.as_ref()).unwrap();
    let overrides: RedisOverrides = serde_json::from_str(&format!(r#"{{"url":{url}}}"#)).unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    Arc::new(connect(options, &BTreeMap::new()).await.unwrap())
}

async fn raw_redis(fixture: &RedisFixture) -> MultiplexedConnection {
    Client::open(fixture.as_ref())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

fn counter_writes(
    redis: &Arc<RedisCache>,
    scope: &Arc<SpendCounterScope>,
) -> SpendCounterWrites<DualCache<LocalMemoryOps>, SqlSpendCounterRows> {
    let memory = LocalMemoryOps::new(1_024, Arc::new(|| 0.0_f64)).unwrap();
    let settings = DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    };
    let batch: Arc<dyn CounterBatch> = Arc::clone(scope);
    SpendCounterWrites {
        layers: Arc::new(DualCache::new(
            Arc::new(memory),
            Some(Arc::clone(redis)),
            settings,
        )),
        batch,
        reseed: Arc::new(SpendCounterReseed {
            rows: None,
            locks: LockRegistry::from(1_000_i64),
            clock: Box::new(UptimeClock {
                started: Instant::now(),
            }),
            durations: Arc::new(NoDurations),
            tasks: TaskTracker::new(),
            gate: Arc::new(LoopBoundSemaphore {
                value: 25,
                slot: Mutex::new(None),
            }),
            stall_tracker: Arc::new(DbLookupStallTracker {
                clock: SystemClock,
                last_hit: Mutex::new(None),
            }),
            deadline_seconds: 5.0_f64,
        }),
        sources: Arc::new(NoSources),
    }
}

async fn spend_logs() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["spend-settlement"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![BASELINE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.config.clone(), NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

fn reserved_user() -> ReservationHandle {
    let entry = ReservationEntry {
        counter_key: Some(COUNTER.to_owned()),
        entity_type: CounterEntityType::User,
        entity_id: "u1".to_owned(),
        reserved_cost: Some(Decimal::from(5_i32)),
        applied_adjustment: Decimal::ZERO,
    };
    ReservationHandle::new(Decimal::from(5_i32), vec![entry], Decimal::ZERO)
}

#[tokio::test]
async fn a_charged_spend_is_persisted_and_settles_the_reserved_redis_counter() {
    let redis_fixture = RedisFixture::start().await.unwrap();
    let redis = redis_cache(&redis_fixture).await;
    let mut connection = raw_redis(&redis_fixture).await;
    cmd("SET")
        .arg(COUNTER)
        .arg("5")
        .query_async::<()>(&mut connection)
        .await
        .unwrap();
    let (postgres, store) = spend_logs().await;
    let table = PostgresSpendLogsWriter {
        store: store.clone(),
    };
    let queue = SpendLogQueue::new(usize::MAX);
    let writer = DbSpendUpdateWriter {
        settings: SpendWriterSettings::default(),
        queues: SpendWriterQueues::default(),
        attachment: Some(SpendLogAttachment {
            table: &table,
            queue: &queue,
        }),
        failures: &NoSpendWriterFailureSink,
        admission: &Admitted,
    };
    let request: SpendUpdateRequest<'_, Context<'_>> = SpendUpdateRequest {
        response_cost: Some(SpendAmount(Decimal::from(2_i32))),
        token: None,
        user_id: Some("u1"),
        end_user_id: None,
        team_id: None,
        org_id: None,
        project_id: None,
        kwargs: None,
        response_names: &[],
        realtime_names: &[],
        model_access_groups: &[],
        turn_environment: None,
    };
    let write = DatabaseSpendWrite {
        writer: &writer,
        payloads: &BatchRow,
        request: &request,
    };
    let scope = Arc::new(SpendCounterScope::default());
    let writes = counter_writes(&redis, &scope);
    let settlement = CounterSettlement {
        backend: SpendCounterBackend {
            writes: &writes,
            scope: scope.as_ref(),
            redis: Some(&redis),
        },
    };
    let services = SpendIncrementServices {
        writes: &writes,
        scope: scope.as_ref(),
        redis: Some(&redis),
        window_queue: &NoWindows,
        counters: &settlement.backend,
    };
    let batch = RequestCounterBatch {
        scope: scope.as_ref(),
        redis: Some(&redis),
    };
    let cache = Recorder::failing(&[]);
    let ports = SettlementPorts {
        reservations: &settlement,
        batch: &batch,
        cache: &cache,
        counters: &services,
    };
    let reservation = reserved_user();
    let input = SettlementInput {
        reservation: Some(&reservation),
        response_cost: Decimal::from(2_i32),
        token: None,
        user_id: Some("u1"),
        end_user_id: None,
        team_id: None,
        org_id: None,
        project_id: None,
        start_time: None,
        tags: None,
        model_access_groups: None,
        update_cache_read_keys: &[],
    };
    assert!(settle_spend_write(&input, &write, &ports).await.unwrap());
    let counter: String = cmd("GET")
        .arg(COUNTER)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(counter.parse::<Decimal>().unwrap(), Decimal::from(2_i32));
    assert!(lock(&reservation.0).finalized);
    let client = store.pool.as_ref().unwrap().get().await.unwrap();
    let row = client
        .query_one(
            r#"SELECT spend::numeric FROM "LiteLLM_SpendLogs" WHERE request_id = $1"#,
            &[&REQUEST_ID],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, Decimal>(0), Decimal::from(2_i32));
    drop(client);
    if let Some(pool) = store.pool {
        pool.close();
    }
    postgres.close_async().await.unwrap();
}
