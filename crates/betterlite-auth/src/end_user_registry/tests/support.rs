#![cfg(test)]

use std::collections::BTreeSet;
use std::future::Future;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{
    DualCache, DualCacheError, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
};
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tracing::Subscriber;
use tracing::subscriber::set_default;

use crate::auth_cache::{AuthCache, AuthCacheError, gateway_cache};
use crate::end_user_registry::cache::RegistryCache;

pub(crate) const CACHE_KEY: &str = "end_user_restricted_registry";

const SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-1', 'tester', 'tester');
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id)
    VALUES ('permission-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, blocked)
    VALUES ('blocked-user', true), ('plain-user', false);
INSERT INTO "LiteLLM_EndUserTable" (user_id, budget_id)
    VALUES ('budget-user', 'budget-1');
INSERT INTO "LiteLLM_EndUserTable" (user_id, allowed_model_region)
    VALUES ('region-user', 'eu'), ('empty-region-user', '');
INSERT INTO "LiteLLM_EndUserTable" (user_id, default_model)
    VALUES ('default-model-user', 'gpt-4o');
INSERT INTO "LiteLLM_EndUserTable" (user_id, object_permission_id)
    VALUES ('permission-user', 'permission-1');
"#;

pub(crate) type TestCache = AuthCache<DualCache<LocalMemoryOps>>;

#[derive(Clone)]
pub(crate) struct Clock(Arc<AtomicU32>);

impl Clock {
    pub(crate) fn new(seconds: u32) -> Self {
        Self(Arc::new(AtomicU32::new(seconds)))
    }

    pub(crate) fn advance(&self, seconds: u32) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }

    pub(crate) fn handle(&self) -> Arc<dyn Fn() -> f64 + Send + Sync> {
        let seconds = Arc::clone(&self.0);
        Arc::new(move || f64::from(seconds.load(Ordering::SeqCst)))
    }
}

pub(crate) fn test_cache(memory_ttl: Option<f64>, clock: &Clock) -> TestCache {
    let memory = || {
        Arc::new(LocalMemoryOps::new(1024, clock.handle()).expect("the local memory tier builds"))
    };
    let settings = DualCacheSettings {
        default_in_memory_ttl: memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    gateway_cache(memory(), memory(), None, settings, 45.0)
}

pub(crate) fn restricted_ids() -> BTreeSet<String> {
    [
        "blocked-user",
        "budget-user",
        "region-user",
        "empty-region-user",
        "default-model-user",
        "permission-user",
    ]
    .map(str::to_owned)
    .into_iter()
    .collect()
}

#[derive(Clone, Copy)]
pub(crate) enum Failure {
    Read,
    Write,
}

pub(crate) struct FailingCache(pub(crate) Failure);

fn backend_failure() -> AuthCacheError {
    let failure = DualCacheError::Memory(BackendError::Unsupported("the registry cache"));
    AuthCacheError::from(failure)
}

#[async_trait]
impl RegistryCache for FailingCache {
    async fn read(&self, _cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        match self.0 {
            Failure::Read => Err(backend_failure()),
            Failure::Write => Ok(None),
        }
    }

    async fn write(
        &self,
        _cache_key: &str,
        _answer: &Json,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        match self.0 {
            Failure::Read => Ok(()),
            Failure::Write => Err(backend_failure()),
        }
    }

    fn management_ttl(&self) -> f64 {
        45.0
    }
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut bytes = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("the log bytes"))?;
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (impl Subscriber + Send + Sync + 'static, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&bytes);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || LogWriter(Arc::clone(&writer)))
        .finish();
    (subscriber, bytes)
}

pub(crate) async fn capture_async<Output, Fut>(operation: Fut) -> (Output, String)
where
    Fut: Future<Output = Output> + Send,
{
    let (subscriber, bytes) = capturing();
    let result = {
        let _guard = set_default(subscriber);
        operation.await
    };
    let text =
        String::from_utf8(bytes.lock().expect("the log bytes").clone()).expect("utf-8 log output");
    (result, text)
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["end-user-registry-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let pool = Pool::builder(Manager::new(
        fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the database url"),
        NoTls,
    ))
    .max_size(2)
    .build()
    .expect("the database pool");
    (fixture, pool)
}

pub(crate) fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

pub(crate) async fn seed(pool: &Pool) {
    let client = pool.get().await.expect("the database connection");
    client.batch_execute(SEED).await.expect("the seeded rows");
}

pub(crate) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture teardown");
}
