#![cfg(test)]

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pool: Pool,
}

impl Database {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["access-group-protocol-tests"])
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
            .max_size(2)
            .build()
            .expect("the fixture pool builds");
        Self { fixture, pool }
    }

    pub(crate) async fn client(&self) -> Client {
        self.pool.get().await.expect("a fixture connection")
    }

    pub(crate) async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        self.client()
            .await
            .execute(statement, parameters)
            .await
            .expect("the fixture statement");
    }

    pub(crate) async fn query_one(
        &self,
        statement: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Row {
        self.client()
            .await
            .query_one(statement, parameters)
            .await
            .expect("the fixture query returns a row")
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}
