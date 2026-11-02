use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use deadpool_postgres::{Client as QuerySession, Manager, Pool};
use futures_util::future::join_all;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::task::{JoinError, JoinSet};
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

struct TestDatabase {
    fixture: PostgresFixture,
    store: PostgresStore,
}

impl TestDatabase {
    async fn session(&self) -> Result<QuerySession, DbError> {
        self.store.session().await
    }

    async fn close(self) {
        if let Some(pool) = &self.store.pool {
            pool.close();
        }
        self.fixture.close_async().await.unwrap();
    }
}

async fn provision() -> Result<TestDatabase, Box<dyn StdError + Send + Sync>> {
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
            return Err(Box::new(error));
        }
        Err(error) => {
            task.close().await?;
            return Err(Box::new(error));
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

fn pool_for(fixture: &PostgresFixture) -> Result<Pool, Box<dyn StdError + Send + Sync>> {
    let config: Config = fixture.options.url.expose_secret().parse()?;
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()?;
    Ok(pool)
}

async fn close_databases(databases: Vec<TestDatabase>) {
    join_all(databases.into_iter().map(TestDatabase::close)).await;
}

async fn provision_all<const N: usize>() -> [TestDatabase; N] {
    let mut databases = Vec::with_capacity(N);
    for _ in 0..N {
        match Box::pin(provision()).await {
            Ok(database) => databases.push(database),
            Err(error) => {
                close_databases(databases).await;
                panic!("failed to provision fixture: {error}");
            }
        }
    }
    databases
        .try_into()
        .unwrap_or_else(|_| panic!("fixture count did not match requested count"))
}

async fn initialize_sessions<const N: usize>(
    databases: &[TestDatabase; N],
) -> Result<[QuerySession; N], Box<dyn StdError + Send + Sync>> {
    let mut sessions = Vec::with_capacity(N);
    for database in databases {
        let session = database.session().await?;
        session.batch_execute(RESET_PUBLIC_SCHEMA_SQL).await?;
        sessions.push(session);
    }
    Ok(sessions.try_into().unwrap())
}

async fn join_body<Fut>(body: Fut) -> Result<(), JoinError>
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut bodies = JoinSet::new();
    bodies.spawn(body);
    let result = timeout(BODY_TIMEOUT, bodies.join_next()).await;
    bodies.shutdown().await;
    match result {
        Ok(Some(outcome)) => outcome,
        Ok(None) => panic!("test body task ended without a result"),
        Err(error) => panic!("test body timed out: {error}"),
    }
}

async fn harness<const N: usize, F, Fut>(body: F)
where
    F: FnOnce([QuerySession; N]) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let databases = Box::pin(provision_all()).await;
    let sessions = match initialize_sessions(&databases).await {
        Ok(sessions) => sessions,
        Err(error) => {
            close_databases(Vec::from(databases)).await;
            panic!("failed to initialize fixture sessions: {error}");
        }
    };
    let outcome = join_body(async move { body(sessions).await }).await;
    close_databases(Vec::from(databases)).await;
    outcome.unwrap();
}
