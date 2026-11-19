#![cfg(test)]

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;
use tokio_postgres::NoTls;

const POOL_SIZE: usize = 2;
const ROW_INSERT: &str =
    r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value") VALUES ($1, to_jsonb($2::text))"#;
const NULL_INSERT: &str =
    r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value") VALUES ($1, NULL)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(crate) async fn provision() -> Self {
        let url = Environment::parse_from(["finops-setup-detection-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![BASELINE.to_owned()])
                .resolve()
                .await
                .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(POOL_SIZE)
            .build()
            .unwrap();
        Self {
            fixture,
            store: PostgresStore { pool: Some(pool) },
            availability: Availability::default(),
        }
    }

    pub(crate) const fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(crate) async fn close(self) {
        self.store.pool.unwrap().close();
        // the closed pool's drivers need a turn to close their sockets before DROP DATABASE
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

pub(crate) fn callbacks(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(crate) async fn seed_value(database: &Database, param: &str, text: &str) {
    let session = database.store.session().await.unwrap();
    session.execute(ROW_INSERT, &[&param, &text]).await.unwrap();
}

pub(crate) async fn seed_null(database: &Database, param: &str) {
    let session = database.store.session().await.unwrap();
    session.execute(NULL_INSERT, &[&param]).await.unwrap();
}

pub(crate) async fn drop_table(database: &Database, table: &str) {
    let session = database.store.session().await.unwrap();
    let statement = format!(r#"DROP TABLE "{table}""#);
    session.batch_execute(&statement).await.unwrap();
}
