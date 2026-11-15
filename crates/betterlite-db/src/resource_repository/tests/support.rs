use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{
    PublishChange, PublishFuture, redis_config_change_publisher,
};
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;
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
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredValue};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use crate::budget_permission_rows::JsonData;
use crate::database_replica::availability::Availability;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;
use crate::resource_repository::table::{ResourceActions, ResourceBinding, RowWrite};
use crate::stored_cipher::DbCipher;
use crate::{
    schema_agent_identity, schema_baseline, schema_budget_windows, schema_daily_usage,
    schema_identity_history, schema_managed_resources, schema_policy_resources,
    schema_routing_state,
};

pub(crate) const NO_DB: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
pub(crate) const CIPHER_KEY: &str = "resource-repository-key";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["resource-repository"])
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
    pub(crate) writer: PostgresStore,
    pub(crate) reader: PostgresStore,
    pub(crate) availability: Availability,
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
        let reader = reader_fixture
            .as_ref()
            .map_or_else(|| writer.clone(), store);
        let separate = reader_fixture.is_some();
        let mut fixtures = vec![writer_fixture];
        fixtures.extend(reader_fixture);
        let case = Self {
            fixtures,
            writer,
            reader,
            availability: Availability::default(),
        };
        migrate(&case.writer).await;
        if separate {
            migrate(&case.reader).await;
        }
        case
    }

    pub(crate) const fn client(&self) -> PrismaClient<'_> {
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
    fn record(&self, entry: &str) {
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
    let entries = journal.clone();
    Arc::new(move |object_type: &str| {
        entries.record(&format!("publish:{object_type}"));
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
    sql: String,
) -> PublishChange<TableError> {
    let entries = journal.clone();
    let pool = store.clone();
    Arc::new(move |object_type: &str| {
        let task_entries = entries.clone();
        let task_pool = pool.clone();
        let name = object_type.to_owned();
        let query = sql.clone();
        let future: PublishFuture<TableError> = Box::pin(async move {
            let client = task_pool.session().await?;
            let row = client
                .query_opt(&query, &[])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            task_entries.record(&format!("publish:{name}:visible={}", row.is_some()));
            Ok(())
        });
        future
    })
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
    let env = BTreeMap::<String, String>::new();
    connect(options, &env).await.unwrap()
}

pub(crate) struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

pub(crate) fn cipher() -> DbCipher {
    cipher_with_key(CIPHER_KEY)
}

pub(crate) fn cipher_with_key(key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn seal_with(text: &str, key: &str) -> String {
    let sealed = cipher_with_key(key)
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.to_owned())),
            None,
        )
        .unwrap();
    let StoredValue::Text(value) = sealed else {
        panic!("a text value stays text");
    };
    value.expose_secret().to_owned()
}

pub(crate) fn seal(text: &str) -> String {
    seal_with(text, CIPHER_KEY)
}

pub(crate) fn value(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn written(text: &str) -> RowWrite {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn write(fields: &[(&str, JsonData)]) -> RowWrite {
    RowWrite(
        fields
            .iter()
            .map(|(name, field)| ((*name).to_owned(), field.clone()))
            .collect(),
    )
}

pub(crate) fn keyed(rows: Vec<JsonData>, column: &str) -> BTreeMap<String, JsonData> {
    rows.into_iter()
        .map(|row| {
            let JsonData::Object(object) = row else {
                panic!("a row is a JSON object");
            };
            let Some(JsonData::String(key)) = object.get(column) else {
                panic!("a row carries {column}");
            };
            (key.clone(), JsonData::Object(object))
        })
        .collect()
}

pub(crate) fn field<'a>(row: &'a JsonData, name: &str) -> &'a JsonData {
    let JsonData::Object(object) = row else {
        panic!("a row is a JSON object");
    };
    object
        .get(name)
        .unwrap_or_else(|| panic!("a row carries {name}"))
}

pub(crate) async fn batch(store: &PostgresStore, sql: &str) {
    let client = store.session().await.unwrap();
    client.batch_execute(sql).await.unwrap();
}

pub(crate) async fn execute(store: &PostgresStore, sql: &str, params: &[&(dyn ToSql + Sync)]) {
    let client = store.session().await.unwrap();
    client.execute(sql, params).await.unwrap();
}

const INSERT_MODEL_SQL: &str = "INSERT INTO \"LiteLLM_ProxyModelTable\" (model_id, model_name, \
     litellm_params, model_info, created_at, updated_at, created_by, updated_by) \
     VALUES ($1, $2, $3::text::jsonb, $4::text::jsonb, '2026-01-02 03:04:05.123', \
     '2026-01-03 04:05:06.789', 'seed', 'seed')";

pub(crate) async fn insert_model(
    case: &Case,
    id: &str,
    name: &str,
    params: &str,
    info: Option<&str>,
) {
    execute(
        &case.writer,
        INSERT_MODEL_SQL,
        &[&id, &name, &params, &info],
    )
    .await;
}

const INSERT_BUDGET_SQL: &str = "INSERT INTO \"LiteLLM_BudgetTable\" (budget_id, max_budget, \
     soft_budget, max_parallel_requests, tpm_limit, rpm_limit, model_max_budget, budget_duration, \
     budget_reset_at, created_by, updated_by) VALUES ($1, 10, 2.5, 3, 40, 50, \
     '{\"gpt-4o\":0.5}', '1d', '2026-01-01 00:00:00', 'seed', 'seed')";

pub(crate) async fn insert_budget(case: &Case, budget_id: &str) {
    execute(&case.writer, INSERT_BUDGET_SQL, &[&budget_id]).await;
}

// The stored value is Prisma's JSON string of the Python str LiteLLM writes.
const INSERT_CONFIG_SQL: &str =
    "INSERT INTO \"LiteLLM_Config\" (param_name, param_value) VALUES ($1, to_jsonb($2::text))";

pub(crate) async fn insert_config(store: &PostgresStore, name: &str, content: &str) {
    execute(store, INSERT_CONFIG_SQL, &[&name, &content]).await;
}

const INSERT_CREDENTIAL_SQL: &str = "INSERT INTO \"LiteLLM_CredentialsTable\" (credential_id, \
     credential_name, credential_values, credential_info, created_by, updated_by) \
     VALUES ($1, $2, $3::text::jsonb, $4::text::jsonb, 'seed', 'seed')";

pub(crate) async fn insert_credential(
    store: &PostgresStore,
    id: &str,
    name: &str,
    values: &str,
    info: Option<&str>,
) {
    execute(store, INSERT_CREDENTIAL_SQL, &[&id, &name, &values, &info]).await;
}

const INSERT_PERMISSION_SQL: &str = "INSERT INTO \"LiteLLM_ObjectPermissionTable\" \
     (object_permission_id, mcp_servers) VALUES ($1, ARRAY[$2]::TEXT[])";

pub(crate) async fn insert_permission(store: &PostgresStore, id: &str, server: &str) {
    execute(store, INSERT_PERMISSION_SQL, &[&id, &server]).await;
}

pub(crate) async fn text_rows(
    store: &PostgresStore,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Vec<String> {
    let client = store.session().await.unwrap();
    client
        .query(sql, params)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

pub(crate) async fn text_row(
    store: &PostgresStore,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> String {
    text_rows(store, sql, params).await.remove(0)
}

// A table whose only columns have server-side defaults, so an empty write exercises DEFAULT VALUES.
pub(crate) struct DefaultsBinding;

impl TableBinding for DefaultsBinding {
    const NAME: &'static str = "resource_repository_defaults";
    const TABLE: &'static str = "ResourceRepositoryDefaults";
}

impl ResourceBinding for DefaultsBinding {
    const KEY: &'static str = "id";
    const ID: Option<&'static str> = None;
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

pub(crate) fn defaults_actions(
    case: &Case,
) -> ConfigSyncTableActions<ResourceActions<'_, DefaultsBinding>> {
    let repository = PrismaTableRepository::new(
        Some(case.client()),
        redis_config_change_publisher(None),
        DefaultsBinding,
    );
    repository.table(ResourceActions::new).unwrap()
}

pub(crate) async fn create_defaults_table(store: &PostgresStore) {
    batch(
        store,
        "CREATE SEQUENCE resource_repository_defaults_seq; \
         CREATE TABLE \"ResourceRepositoryDefaults\" ( \
         \"id\" TEXT NOT NULL DEFAULT ('id-' || nextval('resource_repository_defaults_seq')), \
         \"updated_at\" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP, \
         \"payload\" JSONB, \
         CONSTRAINT \"ResourceRepositoryDefaults_pkey\" PRIMARY KEY (\"id\"))",
    )
    .await;
}
