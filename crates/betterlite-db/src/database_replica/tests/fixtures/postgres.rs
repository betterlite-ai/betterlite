#![cfg(test)]

use std::collections::BTreeMap;
use std::mem::replace;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::{sleep, timeout};
use tokio_postgres::NoTls;

use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::availability::Availability;
use crate::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::routing::RoutingDatabase;

const MODEL: &str = "ReplicaRow";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NativeError {
    #[error("the database URL is invalid")]
    Url {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the database pool could not be built")]
    Build {
        #[source]
        source: deadpool_postgres::BuildError,
    },
    #[error("the database pool could not connect")]
    Open {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("the database pool connection timed out")]
    Timeout,
}

pub(crate) struct Seeded {
    pub(crate) fixture: PostgresFixture,
    pub(crate) url: SecretString,
    pub(crate) pool: Pool,
}

pub(crate) fn pool_for(url: &str, transport: Option<&PoolTransport>) -> Result<Pool, NativeError> {
    let config = url.parse().map_err(|source| NativeError::Url { source })?;
    let mut builder = Pool::builder(Manager::new(config, NoTls));
    if let Some(limits) = transport
        && let Some(max_size) = limits.max_size
    {
        builder = builder.max_size(max_size);
    }
    builder
        .build()
        .map_err(|source| NativeError::Build { source })
}

async fn database_fixture() -> PostgresFixture {
    let configured = Environment::parse_from(["database-replica-tests"])
        .url
        .map(SecretString::from);
    let mut task = PostgresFixture::provision_with_name_generator(
        configured.as_ref(),
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    );
    task.resolve().await.expect("a Postgres database fixture")
}

pub(crate) async fn seeded(note: &str) -> Seeded {
    let fixture = database_fixture().await;
    let url = fixture.options.url.clone();
    let pool = pool_for(url.expose_secret(), None).expect("the fixture pool builds");
    let client = pool.get().await.expect("a fixture connection");
    client
        .batch_execute("CREATE TABLE replica_rows (note text NOT NULL)")
        .await
        .expect("the table is created");
    client
        .execute("INSERT INTO replica_rows VALUES ($1)", &[&note])
        .await
        .expect("the seeded row commits");
    drop(client);
    Seeded { fixture, url, pool }
}

async fn open(pool: &Pool, limit: Option<Duration>) -> Result<(), NativeError> {
    let opened = match limit {
        Some(wait) => timeout(wait, pool.get())
            .await
            .map_err(|_elapsed| NativeError::Timeout)?,
        None => pool.get().await,
    }
    .map_err(|source| NativeError::Open { source })?;
    drop(opened);
    Ok(())
}

// a replaced pool closes asynchronously, so end its backends before the fixture drops the database
pub(crate) async fn close_case(case: Seeded) {
    let pool = pool_for(case.url.expose_secret(), None).expect("the cleanup pool builds");
    let ended = async {
        let client = pool.get().await.ok()?;
        client
            .batch_execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                 WHERE datname = current_database() AND pid <> pg_backend_pid()",
            )
            .await
            .ok()?;
        Some(client)
    };
    let observed = ended.await;
    match observed {
        Some(watcher) => {
            for _attempt in 0_u16..200 {
                let remaining = watcher
                    .query_one(
                        "SELECT count(*) FROM pg_stat_activity \
                         WHERE datname = current_database() AND pid <> pg_backend_pid()",
                        &[],
                    )
                    .await;
                match remaining {
                    Ok(row) if row.get::<_, i64>(0) == 0 => break,
                    _ => sleep(Duration::from_millis(10)).await,
                }
            }
            drop(watcher);
        }
        None => sleep(Duration::from_millis(100)).await,
    }
    pool.close();
    case.pool.close();
    case.fixture
        .close_async()
        .await
        .expect("the fixture database drops");
}

#[derive(Clone)]
pub(crate) struct NativeModel {
    pool: Arc<Mutex<Pool>>,
}

impl NativeModel {
    pub(crate) async fn notes(&self) -> Vec<String> {
        let pool = match self.pool.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let connection = pool.get().await.expect("a pool connection");
        let rows = connection
            .query("SELECT note FROM replica_rows ORDER BY note", &[])
            .await
            .expect("the note query runs");
        rows.iter().map(|row| row.get::<_, String>(0)).collect()
    }
}

pub(crate) struct NativeClient {
    pool: Arc<Mutex<Pool>>,
    connected: Mutex<bool>,
    models: BTreeMap<String, NativeModel>,
}

impl NativeClient {
    pub(crate) fn new(pool: Pool) -> Self {
        let shared = Arc::new(Mutex::new(pool));
        let mut models = BTreeMap::new();
        models.insert(
            MODEL.to_owned(),
            NativeModel {
                pool: Arc::clone(&shared),
            },
        );
        Self {
            pool: shared,
            connected: Mutex::new(false),
            models,
        }
    }

    pub(crate) fn pool(&self) -> Pool {
        match self.pool.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(crate) fn model(&self) -> NativeModel {
        self.models
            .get(MODEL)
            .expect("the model attribute exists")
            .clone()
    }
}

impl DatabaseLifecycle for NativeClient {
    type Error = NativeError;

    fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), NativeError> {
        Box::pin(async move {
            open(&self.pool(), timeout).await?;
            *self.connected.lock().unwrap() = true;
            Ok(())
        })
    }

    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), NativeError> {
        Box::pin(async move {
            self.pool().close();
            *self.connected.lock().unwrap() = false;
            Ok(())
        })
    }

    fn recreate<'database>(
        &'database self,
        url: &'database SecretString,
        transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, NativeError> {
        Box::pin(async move {
            let replacement = pool_for(url.expose_secret(), transport)?;
            open(&replacement, None).await?;
            let previous = match self.pool.lock() {
                Ok(mut guard) => replace(&mut *guard, replacement),
                Err(poisoned) => replace(&mut *poisoned.into_inner(), replacement),
            };
            previous.close();
            *self.connected.lock().unwrap() = true;
            Ok(true)
        })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), NativeError> {
        Box::pin(async { Ok(()) })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), NativeError> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for NativeClient {
    type Attribute = NativeModel;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<NativeModel>>, NativeError> {
        Ok(self
            .models
            .get(name)
            .map(|model| Attribute::Model(model.clone())))
    }

    fn token_refresh_enabled(&self) -> bool {
        false
    }

    fn token_label(&self) -> &'static str {
        "database token"
    }

    fn minted_url(&self) -> Result<Option<SecretString>, NativeError> {
        Ok(None)
    }
}

pub(crate) struct ReplicaCase {
    pub(crate) writer: Seeded,
    pub(crate) reader: Seeded,
}

pub(crate) async fn replica_case(writer_note: &str, reader_note: &str) -> ReplicaCase {
    ReplicaCase {
        writer: seeded(writer_note).await,
        reader: seeded(reader_note).await,
    }
}

pub(crate) fn routing_over(case: &ReplicaCase) -> RoutingDatabase<NativeClient> {
    RoutingDatabase {
        writer: NativeClient::new(case.writer.pool.clone()),
        reader: NativeClient::new(case.reader.pool.clone()),
        availability: Availability::default(),
        reader_url: Some(case.reader.url.clone()),
    }
}

pub(crate) async fn close_replica_case(case: ReplicaCase, routing: &RoutingDatabase<NativeClient>) {
    routing.writer.pool().close();
    routing.reader.pool().close();
    close_case(case.writer).await;
    close_case(case.reader).await;
}

// the server refuses an unknown database, so the reader recreate fails without depending on the network
pub(crate) fn missing_database_url(url: &SecretString) -> SecretString {
    let mut parsed = url::Url::parse(url.expose_secret()).expect("the fixture URL parses");
    parsed.set_path("/betterlite_missing_replica");
    SecretString::from(parsed.to_string())
}
