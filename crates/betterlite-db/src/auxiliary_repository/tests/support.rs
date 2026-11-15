use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_redis::config_sync::table_actions::TableActions;
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
use clap::Parser;
use deadpool_postgres::{GenericClient, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::database_replica::availability::Availability;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;
use crate::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};

use super::super::repository::{PrismaClient, TableError};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["auxiliary-repository"])
        .url
        .map(SecretString::from)
}

async fn provision() -> PostgresFixture {
    PostgresFixture::provision_schema(test_url().as_ref())
        .resolve()
        .await
        .unwrap()
}

pub(crate) fn store(fixture: &PostgresFixture) -> PostgresStore {
    let config = fixture.options.url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    let pool = Pool::builder(manager).max_size(2).build().unwrap();
    PostgresStore { pool: Some(pool) }
}

async fn migrate(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    for catalog in [
        schema_baseline::MIGRATION_CATALOG,
        schema_daily_usage::MIGRATION_CATALOG,
        schema_managed_resources::MIGRATION_CATALOG,
        schema_identity_history::MIGRATION_CATALOG,
        schema_policy_resources::MIGRATION_CATALOG,
        schema_routing_state::MIGRATION_CATALOG,
        schema_budget_windows::MIGRATION_CATALOG,
        schema_agent_identity::MIGRATION_CATALOG,
    ] {
        for migration in catalog.files {
            client
                .batch_execute(from_utf8(migration.bytes).unwrap())
                .await
                .unwrap();
        }
    }
}

pub(crate) struct Case {
    fixtures: Vec<PostgresFixture>,
    pub writer: PostgresStore,
    pub reader: PostgresStore,
    pub availability: Availability,
}

impl Case {
    pub(crate) async fn new() -> Self {
        Self::assemble(provision().await, None).await
    }

    pub(crate) async fn split() -> Self {
        Self::assemble(provision().await, Some(provision().await)).await
    }

    async fn assemble(
        writer_fixture: PostgresFixture,
        reader_fixture: Option<PostgresFixture>,
    ) -> Self {
        let writer = store(&writer_fixture);
        let reader = match &reader_fixture {
            Some(fixture) => store(fixture),
            None => store(&writer_fixture),
        };
        let separate_reader = reader_fixture.is_some();
        let mut fixtures = vec![writer_fixture];
        fixtures.extend(reader_fixture);
        let case = Self {
            fixtures,
            writer,
            reader,
            availability: Availability::default(),
        };
        migrate(&case.writer).await;
        if separate_reader {
            migrate(&case.reader).await;
        }
        case
    }

    pub(crate) fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.writer,
            reader: &self.reader,
            availability: &self.availability,
        }
    }

    pub(crate) async fn close(self) {
        self.writer.pool.unwrap().close();
        self.reader.pool.unwrap().close();
        for fixture in self.fixtures {
            fixture.close_async().await.unwrap();
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(crate) fn record(&self, entry: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.to_owned());
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn recording_publisher(journal: &Journal) -> PublishChange<TableError> {
    let journal = journal.clone();
    Arc::new(move |object_type: &str| {
        journal.record(&format!("publish:{object_type}"));
        let future: PublishFuture<TableError> = Box::pin(async { Ok(()) });
        future
    })
}

pub(crate) fn failing_publisher(message: &'static str) -> PublishChange<TableError> {
    Arc::new(move |_object_type: &str| {
        let future: PublishFuture<TableError> = Box::pin(async move {
            Err(TableError::Publish {
                message: message.to_owned(),
            })
        });
        future
    })
}

pub(crate) fn observing_publisher(
    journal: &Journal,
    store: &PostgresStore,
) -> PublishChange<TableError> {
    let journal = journal.clone();
    let store = store.clone();
    Arc::new(move |object_type: &str| {
        let journal = journal.clone();
        let store = store.clone();
        let name = object_type.to_owned();
        let future: PublishFuture<TableError> = Box::pin(async move {
            let client = store.session().await?;
            let row = client
                .query_opt(
                    "SELECT 1 FROM \"LiteLLM_UISettings\" WHERE id = 'user_banner'",
                    &[],
                )
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            journal.record(&format!("publish:{name}:visible={}", row.is_some()));
            Ok(())
        });
        future
    })
}

async fn execute(store: &PostgresStore, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    let client = store.session().await.unwrap();
    client.execute(sql, params).await.unwrap();
}

pub(crate) async fn insert_batch(
    store: &PostgresStore,
    unified_object_id: &str,
    status: Option<&str>,
    file_object: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    let model_object_id = uuid::Uuid::new_v4().to_string();
    execute(
        store,
        "INSERT INTO \"LiteLLM_ManagedObjectTable\" \
         (id, unified_object_id, model_object_id, file_object, file_purpose, status, updated_at) \
         VALUES ($1, $2, $3, to_jsonb($4::text), 'batch', $5, now())",
        &[
            &id,
            &unified_object_id,
            &model_object_id,
            &file_object,
            &status,
        ],
    )
    .await;
}

pub(crate) async fn set_batch_status(store: &PostgresStore, unified_object_id: &str, status: &str) {
    execute(
        store,
        "UPDATE \"LiteLLM_ManagedObjectTable\" SET status = $2 WHERE unified_object_id = $1",
        &[&unified_object_id, &status],
    )
    .await;
}

pub(crate) async fn drop_batch_unique_index(store: &PostgresStore) {
    let client = store.session().await.unwrap();
    client
        .batch_execute("DROP INDEX \"LiteLLM_ManagedObjectTable_unified_object_id_key\"")
        .await
        .unwrap();
}

pub(crate) async fn insert_session(
    store: &PostgresStore,
    api_key: &str,
    session_id: &str,
    router_name: &str,
    last_model: &str,
    last_turn_at: Option<&str>,
) {
    execute(
        store,
        "INSERT INTO \"LiteLLM_AutoRouterSession\" \
         (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model) \
         VALUES ($1, $2, $3, 'classifier', COALESCE($4::text::timestamp, now()), $4::text::timestamp, $5)",
        &[
            &api_key,
            &session_id,
            &router_name,
            &last_turn_at,
            &last_model,
        ],
    )
    .await;
}

pub(crate) async fn insert_ui_settings(store: &PostgresStore, id: &str, ui_settings: &str) {
    execute(
        store,
        "INSERT INTO \"LiteLLM_UISettings\" (id, ui_settings, updated_at) \
         VALUES ($1, $2::text::jsonb, now())",
        &[&id, &ui_settings],
    )
    .await;
}

pub(crate) fn assert_bindings(present: &BTreeSet<String>) {
    for &(name, table) in super::super::bindings::ALL {
        assert!(present.contains(table), "{name} is backed by {table}");
    }
}

pub(crate) struct RecordingActions;

impl TableActions for RecordingActions {
    type Row = i64;
    type Filter = &'static str;
    type CreateData = &'static str;
    type UpdateData = &'static str;
    type Failure = TableError;

    fn create(
        &self,
        _data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        ready(Ok(1))
    }

    fn create_many(
        &self,
        data: Vec<Self::CreateData>,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(data.len()))
    }

    fn update(
        &self,
        _data: Self::UpdateData,
        _filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        ready(Ok(Some(1)))
    }

    fn update_many(
        &self,
        _data: Self::UpdateData,
        _filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(0))
    }

    fn upsert(
        &self,
        _filter: Self::Filter,
        _data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        ready(Ok(1))
    }

    fn delete(
        &self,
        _filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        ready(Ok(None))
    }

    fn delete_many(
        &self,
        _filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(0))
    }
}

pub(crate) async fn dead_redis() -> RedisCache {
    let overrides: RedisOverrides =
        serde_json::from_str(r#"{"host":"127.0.0.1","port":1}"#).unwrap();
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
    connect(options, &BTreeMap::<String, String>::new())
        .await
        .unwrap()
}

pub(crate) struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn std::error::Error + Send + Sync)) {}
}

pub(crate) struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn std::error::Error + Send + Sync)) {}

    fn swallowed(
        &self,
        _admission: &BreakerAdmission,
        _error: &(dyn std::error::Error + Send + Sync),
    ) {
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        Self::clone(self)
    }
}

pub(crate) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .set_default();
    (logs, guard)
}
