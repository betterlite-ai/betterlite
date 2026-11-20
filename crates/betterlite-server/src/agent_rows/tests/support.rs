#![cfg(test)]

use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::rows::RowCodec as _;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::agent_rows::AGENT_SELECT_SQL;
use crate::agent_rows::codec::{AgentRowCodec, RowDecodeError, StoredAgent};

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
        let url = Environment::parse_from(["agent-row-tests"])
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

    pub(crate) fn store(&self) -> PostgresStore {
        PostgresStore {
            pool: Some(self.pool.clone()),
        }
    }

    pub(crate) async fn run(&self, sql: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client.batch_execute(sql).await.expect("the statements run");
    }

    pub(crate) async fn decode(
        &self,
        agent_id: &str,
    ) -> Result<Option<StoredAgent>, RowDecodeError> {
        let client = self.pool.get().await.expect("a fixture connection");
        let statement = format!("{AGENT_SELECT_SQL} WHERE a.agent_id = $1");
        let maybe_row = client
            .query_opt(&statement, &[&agent_id])
            .await
            .expect("the agent row query runs");
        let Some(row) = maybe_row else {
            return Ok(None);
        };
        AgentRowCodec.decode(&row)
    }

    pub(crate) async fn decode_agent(&self, agent_id: &str) -> StoredAgent {
        self.decode(agent_id)
            .await
            .expect("the agent row decodes")
            .expect("the agent row exists")
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}
