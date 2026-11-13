#![cfg(test)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::sleep;
use tokio_postgres::NoTls;

use crate::auth_reconnect::clock::SystemClock;
use crate::auth_reconnect::engine::{EngineState, ReconnectEngine};
use crate::auth_reconnect::lock::TokioLock;
use crate::auth_reconnect::policy::AuthReconnect;
use crate::auth_reconnect::probe::{WriterProbeError, writer_is_read_only, writer_select_one};
use crate::auth_reconnect::request::{ReconnectHistory, ReconnectRequest};
use crate::auth_reconnect::settings::ReconnectSettings;
use crate::database_refresh::generation::Observation;
use crate::database_refresh::handle::PoolFuture;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn fixture() -> PostgresFixture {
    let configured = Environment::parse_from(["auth-reconnect-tests"])
        .url
        .map(SecretString::from);
    let mut task = PostgresFixture::provision_with_name_generator(
        configured.as_ref(),
        &FixtureSeed::default(),
        &mut RandomDatabaseName,
    );
    task.resolve().await.expect("a Postgres database fixture")
}

#[derive(Debug, thiserror::Error)]
enum NativeError {
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
    #[error("the writer probe failed")]
    Probe {
        #[source]
        source: WriterProbeError,
    },
}

fn build_pool(url: &str) -> Result<Pool, NativeError> {
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|source| NativeError::Url { source })?;
    Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .map_err(|source| NativeError::Build { source })
}

struct NativeShared {
    pool: Mutex<Arc<Pool>>,
    generation: AtomicU64,
    dead: AtomicBool,
}

#[derive(Clone)]
struct NativeEngine {
    shared: Arc<NativeShared>,
}

impl NativeEngine {
    fn new(pool: Pool) -> Self {
        Self {
            shared: Arc::new(NativeShared {
                pool: Mutex::new(Arc::new(pool)),
                generation: AtomicU64::new(0),
                dead: AtomicBool::new(false),
            }),
        }
    }

    fn pool(&self) -> Arc<Pool> {
        Arc::clone(
            &self
                .shared
                .pool
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    fn mark_dead(&self) {
        self.shared.dead.store(true, Ordering::SeqCst);
    }

    fn is_dead(&self) -> bool {
        self.shared.dead.load(Ordering::SeqCst)
    }
}

impl ReconnectEngine for NativeEngine {
    type Handle = Pool;
    type Error = NativeError;

    fn writer(&self) -> Observation<Self::Handle> {
        Observation {
            generation: self.generation(),
            handle: Some(self.pool()),
            connected: true,
        }
    }

    fn read_target(&self) -> Observation<Self::Handle> {
        self.writer()
    }

    fn engine_state(&self) -> EngineState {
        if self.is_dead() {
            EngineState::Dead
        } else {
            EngineState::Alive
        }
    }

    fn recreate(
        &self,
        url: &SecretString,
        expected_generation: Option<u64>,
    ) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            if let Some(expected) = expected_generation
                && expected != self.generation()
            {
                return Ok(false);
            }
            let replacement = build_pool(url.expose_secret())?;
            let connection = replacement
                .get()
                .await
                .map_err(|source| NativeError::Open { source })?;
            drop(connection);
            let previous = {
                let mut pool = self
                    .shared
                    .pool
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                std::mem::replace(&mut *pool, Arc::new(replacement))
            };
            previous.close();
            self.shared.generation.fetch_add(1, Ordering::SeqCst);
            self.shared.dead.store(false, Ordering::SeqCst);
            Ok(true)
        })
    }

    fn writer_read_only(&self) -> PoolFuture<'_, bool, Self::Error> {
        Box::pin(async move {
            writer_is_read_only(&self.pool())
                .await
                .map_err(|source| NativeError::Probe { source })
        })
    }

    fn writer_select_one(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            writer_select_one(&self.pool())
                .await
                .map_err(|source| NativeError::Probe { source })
        })
    }

    fn mark_writer_recovered(&self) {}

    fn reap(&self) {}

    fn cleanup_watcher(&self) {}

    fn rearm_watcher(&self) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async { Ok(()) })
    }
}

async fn close_backends(url: &SecretString) {
    let Ok(pool) = build_pool(url.expose_secret()) else {
        return;
    };
    let Ok(client) = pool.get().await else {
        pool.close();
        return;
    };
    let _terminated = client
        .batch_execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = current_database() AND pid <> pg_backend_pid()",
        )
        .await;
    for _attempt in 0_u16..200 {
        let remaining = client
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
    drop(client);
    pool.close();
}

#[tokio::test]
async fn postgres_writer_read_only_probe() {
    let fixture = fixture().await;
    let url = fixture.options.url.clone();
    let database = {
        let pool = build_pool(url.expose_secret()).expect("the fixture pool builds");
        let client = pool.get().await.expect("a fixture connection");
        let row = client
            .query_one("SELECT current_database()", &[])
            .await
            .expect("the database name is readable");
        pool.close();
        row.get::<_, String>(0)
    };
    let writer = build_pool(url.expose_secret()).expect("the writer pool builds");
    assert!(
        !writer_is_read_only(&writer)
            .await
            .expect("the writable session answers")
    );
    let client = writer.get().await.expect("a writer connection");
    client
        .batch_execute(&format!(
            "ALTER DATABASE \"{database}\" SET default_transaction_read_only = on"
        ))
        .await
        .expect("the database turns read-only");
    drop(client);
    writer.close();
    assert!(writer_select_one(&writer).await.is_err());
    let read_only = build_pool(url.expose_secret()).expect("the read-only pool builds");
    assert!(
        writer_is_read_only(&read_only)
            .await
            .expect("the read-only session answers")
    );
    read_only.close();
    close_backends(&url).await;
    fixture
        .close_async()
        .await
        .expect("the fixture database drops");
}

#[tokio::test]
async fn postgres_reconnect_recovery() {
    let fixture = fixture().await;
    let url = fixture.options.url.clone();
    let pool = build_pool(url.expose_secret()).expect("the fixture pool builds");
    let client = pool.get().await.expect("the fixture pool connects");
    drop(client);
    let engine = NativeEngine::new(pool);
    engine.pool().close();
    engine.mark_dead();
    let policy = AuthReconnect::new(
        engine.clone(),
        ReconnectSettings::default(),
        SystemClock,
        TokioLock::default(),
        ReconnectHistory::default(),
    );
    let attempt = ReconnectRequest {
        reason: "postgres_recovery",
        force: true,
        force_recreate: false,
        timeout: Some(Duration::from_secs(10)),
        lock_timeout: Some(Duration::from_secs(10)),
        database_url: Some(&url),
    };
    assert!(policy.attempt_db_reconnect(&attempt, None).await);
    assert_eq!(engine.generation(), 1);
    assert!(!engine.is_dead());
    writer_select_one(&engine.pool())
        .await
        .expect("the recreated pool answers SELECT 1");
    engine.pool().close();
    close_backends(&url).await;
    fixture
        .close_async()
        .await
        .expect("the fixture database drops");
}
