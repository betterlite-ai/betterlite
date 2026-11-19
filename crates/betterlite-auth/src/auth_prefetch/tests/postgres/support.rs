#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::auth_prefetch::fill::PrefetchClock;
use crate::auth_prefetch::memory::{MemoryWriteError, PrefetchCache, PrefetchMemory};
use crate::auth_prefetch::redis_rows::PrefetchRedis;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Database {
    pub(super) fixture: PostgresFixture,
    pub(super) pool: Pool,
    pub(super) store: PostgresStore,
}

pub(super) async fn database() -> Database {
    let url = Environment::parse_from(["auth-prefetch-postgres-tests"])
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
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    Database {
        fixture,
        pool,
        store,
    }
}

pub(super) async fn seed(pool: &Pool, statements: &str) {
    let client = pool.get().await.expect("the database connection");
    client
        .batch_execute(statements)
        .await
        .expect("the seeded rows");
}

pub(super) async fn close(database: Database) {
    database.pool.close();
    database
        .fixture
        .close_async()
        .await
        .expect("the fixture teardown");
}

pub(super) fn field<'row>(row: &'row JsonData, name: &str) -> &'row JsonData {
    let JsonData::Object(fields) = row else {
        panic!("the row is not an object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the row has no {name} column"))
}

pub(super) fn is_null(row: &JsonData, name: &str) -> bool {
    matches!(field(row, name), JsonData::Null)
}

pub(super) fn strings(row: &JsonData, name: &str) -> Vec<String> {
    let JsonData::Array(items) = field(row, name) else {
        panic!("{name} is not an array");
    };
    items.iter().map(|item| text(item, name)).collect()
}

pub(super) fn text(row: &JsonData, name: &str) -> String {
    match field(row, name) {
        JsonData::String(text) => text.clone(),
        other @ (JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_)) => panic!("{name} is not a string: {other:?}"),
    }
}

pub(super) fn assert_empty_arrays(row: &JsonData, names: &[&str]) {
    for name in names {
        assert!(
            strings(row, name).is_empty(),
            "{name} does not normalize to []"
        );
    }
}

pub(super) fn refreshed_at(
    writes: &[(String, CacheValue, Option<f64>)],
    key: &str,
) -> Option<CacheValue> {
    let write = writes.iter().find(|write| write.0 == key)?;
    let CacheValue::Object(fields) = &write.1 else {
        panic!("the prefetched write holds an object");
    };
    fields.get("last_refreshed_at").cloned()
}

pub(super) fn object(row: &JsonData, name: &str) -> JsonData {
    field(row, name).clone()
}

// the write-back half of the prefetch, with no redis and no TTL of its own
#[derive(Default)]
pub(super) struct CountingCache {
    values: Mutex<BTreeMap<String, CacheValue>>,
    writes: Mutex<Vec<(String, CacheValue, Option<f64>)>>,
}

impl CountingCache {
    pub(super) fn writes(&self) -> Vec<(String, CacheValue, Option<f64>)> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PrefetchMemory for CountingCache {
    fn get(&self, key: &str) -> Option<CacheValue> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, value: CacheValue, ttl: Option<f64>) -> Result<(), MemoryWriteError> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value.clone(), ttl));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
        Ok(())
    }
}

impl PrefetchCache for CountingCache {
    fn ordinary_memory(&self) -> &dyn PrefetchMemory {
        self
    }

    fn memory_for(&self, _key: &str) -> &dyn PrefetchMemory {
        self
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }

    fn default_memory_ttl(&self) -> Option<f64> {
        None
    }

    fn redis(&self) -> Option<Arc<dyn PrefetchRedis>> {
        None
    }
}

#[derive(Default)]
pub(super) struct CountingClock {
    pub(super) now: f64,
    calls: AtomicUsize,
}

impl CountingClock {
    pub(super) const fn reading(now: f64) -> Self {
        Self {
            now,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PrefetchClock for CountingClock {
    fn now(&self) -> f64 {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.now
    }
}
