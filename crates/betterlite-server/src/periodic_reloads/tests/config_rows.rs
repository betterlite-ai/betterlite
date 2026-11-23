use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_refresh::handle::{PoolFuture, PoolTransport};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::key_user_writes::retry::SystemWriteClock;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use betterlite_db::reload_schedule::evictor::{ConfigParamEvictor, EvictFuture};
use betterlite_db::reload_schedule::store::ScheduleDeadlines;
use betterlite_db::table_ops::access::CONFIG_ATTRIBUTE;
use betterlite_db::table_ops::{
    ConfigParamInvalidator, FailureFuture, FailureSink, InvalidateFuture, TableFailureEvent,
    TableOps, TablePorts, TableSettings,
};
use betterlite_db::tool_spend::TokioPause;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser as _;
use deadpool_postgres::{GenericClient as _, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;
use tokio_postgres::NoTls;

use crate::config_param_cache::CONFIG_CACHE_PREFIX;
use crate::config_param_cache::ConfigParamCache;
use crate::config_param_cache::store::TableConfigParamStore;
use crate::periodic_reloads::config_rows::{DatabaseReloadConfigRows, ReloadConfigRows as _};

use super::support::moment;

const BETA_PARAM_NAME: &str = "anthropic_beta_headers_reload_config";
const POOL_SIZE: usize = 4;

const SEED_SQL: &str = r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value", "last_run_at", "reload_revision")
VALUES ($1, $2::text::jsonb, $3, $4)"#;

const READ_RUN_SQL: &str = r#"SELECT "last_run_at" FROM "LiteLLM_Config" WHERE "param_name" = $1"#;

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct TableClient {
    pool: Pool,
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
        if name == CONFIG_ATTRIBUTE {
            return Ok(Some(Attribute::Model(self.pool.clone())));
        }
        Ok(None)
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

struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Default)]
struct SilentSink;

impl FailureSink<Infallible> for SilentSink {
    fn record(&self, _event: TableFailureEvent<'_, Infallible>) -> FailureFuture<'_> {
        Box::pin(async {})
    }
}

#[derive(Default)]
struct NoopInvalidator;

impl ConfigParamInvalidator for NoopInvalidator {
    fn invalidate_param<'call>(&'call self, _param_name: &'call str) -> InvalidateFuture<'call> {
        Box::pin(async {})
    }
}

struct Ports {
    clock: SystemWriteClock,
    jitter: ZeroJitter,
    pause: TokioPause,
    sink: SilentSink,
    invalidator: NoopInvalidator,
}

impl Ports {
    fn new() -> Self {
        Self {
            clock: SystemWriteClock,
            jitter: ZeroJitter,
            pause: TokioPause,
            sink: SilentSink,
            invalidator: NoopInvalidator,
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

#[derive(Clone, Default)]
struct RecordingEvictor {
    names: Arc<Mutex<Vec<String>>>,
}

impl RecordingEvictor {
    fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ConfigParamEvictor for RecordingEvictor {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call> {
        let names = Arc::clone(&self.names);
        let name = param_name.to_owned();
        Box::pin(async move {
            names
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(name);
            Ok(())
        })
    }
}

fn memory_cache() -> DualCache<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    let memory = Arc::new(LocalMemoryOps::new(1024, clock).expect("the memory tier builds"));
    DualCache::new(
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
    )
}

struct Harness {
    fixture: PostgresFixture,
    pool: Pool,
    table_client: TableClient,
    store: PostgresStore,
    broken: PostgresStore,
    availability: Availability,
    ports: Ports,
    evictor: RecordingEvictor,
    dual: DualCache<LocalMemoryOps>,
}

impl Harness {
    async fn start() -> Self {
        let url = Environment::parse_from(["periodic-reload-rows-tests"])
            .url
            .map(SecretString::from);
        let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the catalogs are UTF-8");
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
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
            .max_size(POOL_SIZE)
            .build()
            .expect("the fixture pool builds");
        let table_client = TableClient { pool: pool.clone() };
        let store = PostgresStore {
            pool: Some(pool.clone()),
        };
        Self {
            fixture,
            pool,
            table_client,
            store,
            broken: PostgresStore { pool: None },
            availability: Availability::default(),
            ports: Ports::new(),
            evictor: RecordingEvictor::default(),
            dual: memory_cache(),
        }
    }

    fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    fn writer_only_client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.broken,
            availability: &self.availability,
        }
    }

    fn operations(&self) -> TableOps<'_, TableClient> {
        TableOps {
            database: Some(WriterPinnedClient::Writer(&self.table_client)),
            ports: self.ports.table_ports(),
            settings: TableSettings::default(),
        }
    }

    fn cache(&self) -> ConfigParamCache<'_, LocalMemoryOps> {
        ConfigParamCache {
            cache: &self.dual,
            ttl_seconds: 60.0,
        }
    }

    async fn cached(&self, key: &str) -> Option<Json> {
        self.dual.async_get_cache(key, false).await
    }

    async fn seed_schedule(
        &self,
        interval_hours: Option<u32>,
        last_run_at: Option<NaiveDateTime>,
        revision: i64,
    ) {
        let interval = interval_hours.map_or(JsonData::Null, |hours| {
            JsonData::Number(serde_json::Number::from(hours))
        });
        let value = JsonData::Object(BTreeMap::from([("interval_hours".to_owned(), interval)]));
        let encoded = serde_json::to_string(&value).expect("the schedule value encodes");
        let session = self.store.session().await.expect("a fixture connection");
        session
            .execute(
                SEED_SQL,
                &[&RELOAD_CONFIG_PARAM_NAME, &encoded, &last_run_at, &revision],
            )
            .await
            .expect("the schedule row commits");
    }

    async fn last_run_at(&self) -> Option<NaiveDateTime> {
        let session = self.store.session().await.expect("a fixture connection");
        let row = session
            .query_one(READ_RUN_SQL, &[&RELOAD_CONFIG_PARAM_NAME])
            .await
            .expect("the schedule row exists");
        row.get(0)
    }

    async fn close(self) {
        self.pool.close();
        // the closed pool's drivers need a turn to close their sockets before DROP DATABASE
        yield_now().await;
        self.fixture
            .close_async()
            .await
            .expect("the fixture database drops");
    }
}

#[tokio::test]
async fn the_beta_row_is_written_through_the_writer_and_evicted_from_the_cache() {
    let harness = Harness::start().await;
    let cache = harness.cache();
    let operations = harness.operations();
    let store = TableConfigParamStore {
        operations: &operations,
    };
    let adapter = DatabaseReloadConfigRows {
        client: Some(harness.writer_only_client()),
        evictor: &harness.evictor,
        deadlines: ScheduleDeadlines::default(),
        cache: &cache,
        store: &store,
    };
    let value = JsonData::Object(BTreeMap::from([
        ("force_reload".to_owned(), JsonData::Bool(false)),
        (
            "interval_hours".to_owned(),
            JsonData::Number(serde_json::Number::from(5_u32)),
        ),
    ]));
    adapter
        .write_beta_headers(value.clone())
        .await
        .expect("the upsert reaches the writer even though the reader is missing");
    let stored = adapter
        .read_beta_headers()
        .await
        .expect("the row read runs")
        .expect("the row exists");
    assert_eq!(stored.param_name, BETA_PARAM_NAME);
    assert_eq!(stored.param_value, value);
    let key = format!("{CONFIG_CACHE_PREFIX}{BETA_PARAM_NAME}");
    assert!(harness.cached(&key).await.is_some());
    adapter
        .evict_beta_headers()
        .await
        .expect("the eviction runs");
    assert!(harness.cached(&key).await.is_none());
    harness.close().await;
}

#[tokio::test]
async fn the_schedule_row_is_read_from_the_database() {
    let harness = Harness::start().await;
    harness.seed_schedule(Some(5), None, 3).await;
    let cache = harness.cache();
    let operations = harness.operations();
    let store = TableConfigParamStore {
        operations: &operations,
    };
    let adapter = DatabaseReloadConfigRows {
        client: Some(harness.client()),
        evictor: &harness.evictor,
        deadlines: ScheduleDeadlines::default(),
        cache: &cache,
        store: &store,
    };
    let schedule = adapter
        .read_schedule()
        .await
        .expect("the schedule read runs")
        .expect("the row exists");
    assert_eq!(schedule.interval_hours, Some(5));
    assert_eq!(schedule.reload_revision, 3);
    assert_eq!(schedule.last_run_at, None);
    harness.close().await;
}

#[tokio::test]
async fn recording_a_run_stamps_the_schedule_row_and_evicts_it() {
    let harness = Harness::start().await;
    harness.seed_schedule(None, None, 1).await;
    let cache = harness.cache();
    let operations = harness.operations();
    let store = TableConfigParamStore {
        operations: &operations,
    };
    let adapter = DatabaseReloadConfigRows {
        client: Some(harness.client()),
        evictor: &harness.evictor,
        deadlines: ScheduleDeadlines::default(),
        cache: &cache,
        store: &store,
    };
    let ran_at = moment(2);
    adapter.record_run(ran_at).await.expect("the run records");
    assert_eq!(harness.last_run_at().await, Some(ran_at.naive_utc()));
    assert_eq!(
        harness.evictor.names(),
        vec![RELOAD_CONFIG_PARAM_NAME.to_owned()]
    );
    harness.close().await;
}
