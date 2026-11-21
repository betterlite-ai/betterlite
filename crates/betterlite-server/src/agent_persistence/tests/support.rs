#![cfg(test)]

use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;

use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::PatchAgentRequest;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::rows::RowCodec as _;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::structured::SensitiveDataMasker;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::agent_persistence::AgentChangePublisher;
use crate::agent_persistence::AgentStore;
use crate::agent_rows::AGENT_SELECT_SQL;
use crate::agent_rows::codec::AgentRowCodec;
use crate::agent_rows::codec::StoredAgent;

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
        let url = Environment::parse_from(["agent-persistence-tests"])
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

    pub(crate) async fn execute(&self, sql: &str, parameters: &[&(dyn ToSql + Sync)]) -> u64 {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(sql, parameters)
            .await
            .expect("the statement runs")
    }

    pub(crate) async fn count(&self, sql: &str, args: &[&(dyn ToSql + Sync)]) -> i64 {
        let client = self.pool.get().await.expect("a fixture connection");
        let query = client.query_one(sql, args);
        let row = query.await.expect("the count query runs");
        row.get(0)
    }

    pub(crate) async fn text(&self, sql: &str, args: &[&(dyn ToSql + Sync)]) -> Option<String> {
        let client = self.pool.get().await.expect("a fixture connection");
        let query = client.query_opt(sql, args);
        let row = query.await.expect("the value query runs")?;
        row.get(0)
    }

    pub(crate) async fn json(&self, sql: &str, args: &[&(dyn ToSql + Sync)]) -> Option<JsonData> {
        let text = self.text(sql, args).await?;
        let decoded = serde_json::from_str(&text).expect("the stored JSON decodes");
        Some(decoded)
    }

    pub(crate) async fn decode_agent(&self, agent_id: &str) -> StoredAgent {
        let client = self.pool.get().await.expect("a fixture connection");
        let statement = format!("{AGENT_SELECT_SQL} WHERE a.agent_id = $1");
        let row = client
            .query_opt(&statement, &[&agent_id])
            .await
            .expect("the agent row query runs")
            .expect("the agent row exists");
        AgentRowCodec
            .decode(&row)
            .expect("the agent row decodes")
            .expect("the agent row decodes to an agent")
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingPublisher {
    calls: Arc<Mutex<usize>>,
}

impl RecordingPublisher {
    pub(crate) fn calls(&self) -> usize {
        *self.calls.lock().expect("the counter lock")
    }
}

impl AgentChangePublisher for RecordingPublisher {
    fn agents_changed(&self) -> impl Future<Output = ()> + Send {
        let calls = Arc::clone(&self.calls);
        async move {
            *calls.lock().expect("the counter lock") += 1;
        }
    }
}

pub(crate) fn store_with(
    database: &Database,
    publisher: RecordingPublisher,
) -> AgentStore<RecordingPublisher> {
    AgentStore {
        store: database.store(),
        masker: SensitiveDataMasker::default(),
        max_json_depth: 100,
        publisher,
    }
}

pub(crate) fn agent(json: &str) -> AgentConfig {
    serde_json::from_str(json).expect("a valid agent config")
}

pub(crate) fn patch(json: &str) -> PatchAgentRequest {
    serde_json::from_str(json).expect("a valid agent patch")
}

pub(crate) const IDENTITY: &str = r#""identity": {"provider": "microsoft_entra", "tenant_id": "11111111-1111-1111-1111-111111111111", "client_id": "22222222-2222-2222-2222-222222222222", "service_principal_id": "33333333-3333-3333-3333-333333333333"}"#;
