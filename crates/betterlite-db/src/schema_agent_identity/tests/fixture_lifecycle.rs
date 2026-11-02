use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::error::FixtureError;
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{BuildError, Client as QuerySession, Manager, Pool};
use futures_util::future::join_all;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::{Future, pending};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::task::JoinError;
use tokio::task::JoinSet;
use tokio::time::error::Elapsed;
use tokio::time::timeout;
use tokio_postgres::{Config, NoTls};

use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SETUP_TIMEOUT: Duration = Duration::from_secs(30);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const RESET_PUBLIC_SCHEMA_SQL: &str = "DROP SCHEMA public CASCADE; CREATE SCHEMA public;";

#[derive(Debug, thiserror::Error)]
enum HarnessError {
    #[error("PostgreSQL fixture operation failed: {0}")]
    Fixture(#[from] FixtureError),
    #[error("PostgreSQL session acquisition failed: {0}")]
    Session(#[from] DbError),
    #[error("PostgreSQL pool construction failed: {0}")]
    Pool(#[from] BuildError),
    #[error("PostgreSQL query failed: {0}")]
    Database(#[from] tokio_postgres::Error),
    #[error("{operation} timed out: {source}")]
    Timeout {
        operation: &'static str,
        #[source]
        source: Elapsed,
    },
    #[error("test body task failed: {0}")]
    Task(#[from] JoinError),
    #[error("test body task ended without a result")]
    MissingTaskResult,
}

struct TestDatabase {
    fixture: PostgresFixture,
    store: PostgresStore,
}

impl TestDatabase {
    async fn close(self) -> Result<(), HarnessError> {
        if let Some(pool) = &self.store.pool {
            pool.close();
        }
        self.fixture.close_async().await?;
        Ok(())
    }
}

async fn provision() -> Result<TestDatabase, HarnessError> {
    let url = Environment::parse_from(["schema-agent-identity"])
        .url
        .map(SecretString::from);
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        url.as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    let fixture = match timeout(SETUP_TIMEOUT, task.resolve()).await {
        Ok(Ok(fixture)) => fixture,
        Ok(Err(error)) => {
            task.close().await?;
            return Err(HarnessError::Fixture(error));
        }
        Err(source) => {
            task.close().await?;
            return Err(HarnessError::Timeout {
                operation: "fixture provisioning",
                source,
            });
        }
    };
    let pool = match pool_for(&fixture) {
        Ok(pool) => pool,
        Err(error) => {
            fixture.close_async().await?;
            return Err(error);
        }
    };
    Ok(TestDatabase {
        fixture,
        store: PostgresStore { pool: Some(pool) },
    })
}

fn pool_for(fixture: &PostgresFixture) -> Result<Pool, HarnessError> {
    let config: Config = fixture.options.url.expose_secret().parse()?;
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()?;
    Ok(pool)
}

async fn close_databases(databases: Vec<TestDatabase>) -> Result<(), HarnessError> {
    let outcomes = join_all(databases.into_iter().map(TestDatabase::close)).await;
    for outcome in outcomes {
        outcome?;
    }
    Ok(())
}

async fn provision_all<const N: usize>() -> Result<[TestDatabase; N], HarnessError> {
    let mut databases = Vec::with_capacity(N);
    for _ in 0..N {
        match Box::pin(provision()).await {
            Ok(database) => databases.push(database),
            Err(error) => {
                close_databases(databases).await?;
                return Err(error);
            }
        }
    }
    Ok(databases
        .try_into()
        .unwrap_or_else(|_| panic!("fixture count did not match requested count")))
}

async fn initialize_sessions<const N: usize>(
    databases: &[TestDatabase; N],
) -> Result<[QuerySession; N], HarnessError> {
    let mut sessions = Vec::with_capacity(N);
    for database in databases {
        let session = timeout(SETUP_TIMEOUT, database.store.session())
            .await
            .map_err(|source| HarnessError::Timeout {
                operation: "session acquisition",
                source,
            })??;
        timeout(
            SETUP_TIMEOUT,
            session.batch_execute(RESET_PUBLIC_SCHEMA_SQL),
        )
        .await
        .map_err(|source| HarnessError::Timeout {
            operation: "schema initialization",
            source,
        })??;
        sessions.push(session);
    }
    Ok(sessions
        .try_into()
        .unwrap_or_else(|_| panic!("session count did not match requested count")))
}

async fn join_body<Fut>(body: Fut, duration: Duration) -> Result<(), HarnessError>
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut bodies = JoinSet::new();
    bodies.spawn(body);
    let outcome = match timeout(duration, bodies.join_next()).await {
        Ok(Some(result)) => result.map_err(HarnessError::Task),
        Ok(None) => Err(HarnessError::MissingTaskResult),
        Err(source) => Err(HarnessError::Timeout {
            operation: "test body",
            source,
        }),
    };
    bodies.shutdown().await;
    outcome
}

async fn body_with_cleanup<Fut, CleanFut>(
    body: Fut,
    duration: Duration,
    cleanup: CleanFut,
) -> Result<(), HarnessError>
where
    Fut: Future<Output = ()> + Send + 'static,
    CleanFut: Future<Output = Result<(), HarnessError>>,
{
    let body_result = join_body(body, duration).await;
    let cleanup_result = cleanup.await;
    match cleanup_result {
        Err(error) => Err(error),
        Ok(()) => body_result,
    }
}

async fn harness<const N: usize, F, Fut>(body: F)
where
    F: FnOnce([QuerySession; N]) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let databases = match Box::pin(provision_all()).await {
        Ok(databases) => databases,
        Err(error) => panic!("failed to provision fixtures: {error}"),
    };
    let sessions = match initialize_sessions(&databases).await {
        Ok(sessions) => sessions,
        Err(error) => {
            close_databases(Vec::from(databases))
                .await
                .unwrap_or_else(|cleanup_error| panic!("fixture cleanup failed: {cleanup_error}"));
            panic!("failed to initialize fixture sessions: {error}");
        }
    };
    let cleanup = close_databases(Vec::from(databases));
    body_with_cleanup(async move { body(sessions).await }, BODY_TIMEOUT, cleanup)
        .await
        .unwrap_or_else(|error| panic!("PostgreSQL test failed: {error}"));
}

#[tokio::test]
async fn body_timeout_runs_fixture_cleanup() {
    let cleanup_ran = Arc::new(AtomicBool::new(false));
    let cleanup_signal = Arc::clone(&cleanup_ran);
    let cleanup = async move {
        cleanup_signal.store(true, Ordering::SeqCst);
        Ok(())
    };
    let body = pending::<()>();
    let result = body_with_cleanup(body, Duration::from_millis(1), cleanup).await;
    assert!(matches!(result, Err(HarnessError::Timeout { .. })));
    assert!(cleanup_ran.load(Ordering::SeqCst));
}
