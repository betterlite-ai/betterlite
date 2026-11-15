use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistryApi as _;

pub(crate) const ALPHA_ENTRY: &str =
    r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#;
pub(crate) const ALPHA_STABLE: &str =
    "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8";
pub(crate) const ALPHA_LEGACY: &str =
    "00afdb1e315efd7d8d96382f3925c7d46dee40711cffe80449b0dcb21f731322";
pub(crate) const BETA_ENTRY: &str = r#"{"agent_name":"béta","agent_card_params":{"modèle":"x\u007fy","emoji":"😀","raw":"a\n\t\r\b\f\"\\/"}}"#;
pub(crate) const BETA_STABLE: &str =
    "635891353974cd5ae4427fe3ea7ed477e8b74f703ad175fa74d29a7c96e869f5";
pub(crate) const BETA_LEGACY: &str =
    "ab79e4913c6aaed7238fce84ab6b3c6db0f27cab4e6e0549dd2061e7e8c5ec17";
pub(crate) const FLOATS_ENTRY: &str = r#"{"agent_name":"floats","agent_card_params":{"a":1,"b":1.0,"c":1e16,"d":1e-5,"e":1.5e-7,"f":100000.0,"g":0.0001,"h":-0.0,"i":1.2345678901234567,"j":5e-324,"nested":[true,false,null,{"z":1,"A":[2,{"é":3}]}]}}"#;
pub(crate) const FLOATS_STABLE: &str =
    "4b379441613442e9bd746f463554d9e27aa22d4d4aa4aa7c84f3192997bc2200";
pub(crate) const FLOATS_LEGACY: &str =
    "da711e04632a269baea7837b076f61e3309bd15ae9c5353ce63650ee4434e55a";

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ObjectPermissionTable" (
    object_permission_id TEXT PRIMARY KEY,
    agents TEXT[] NOT NULL DEFAULT '{}'
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct TestDatabase {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
}

impl TestDatabase {
    pub(crate) async fn start(schema: &str) -> Self {
        let url = Environment::parse_from(["betterlite-auth-agent-registry"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .unwrap();
        let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
        let pool = Pool::builder(manager).max_size(4).build().unwrap();
        let client = pool.get().await.unwrap();
        client.batch_execute(schema).await.unwrap();
        drop(client);
        Self { fixture, pool }
    }

    pub(crate) fn store(&self) -> PostgresStore {
        let pool = self.pool.clone();
        PostgresStore { pool: Some(pool) }
    }

    pub(crate) async fn close(self) {
        let Self { fixture, pool } = self;
        pool.close();
        fixture.close_async().await.unwrap();
    }
}

pub(crate) async fn insert_row(pool: &Pool, object_permission_id: &str, agents: &[String]) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, agents) VALUES ($1, $2)"#,
            &[&object_permission_id, &agents],
        )
        .await
        .unwrap();
}

pub(crate) async fn row_agents(pool: &Pool, object_permission_id: &str) -> Option<Vec<String>> {
    let client = pool.get().await.unwrap();
    let row = client
        .query_opt(
            r#"SELECT agents FROM "LiteLLM_ObjectPermissionTable" WHERE object_permission_id = $1"#,
            &[&object_permission_id],
        )
        .await
        .unwrap();
    row.map(|found| found.get(0))
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn entries(texts: &[&str]) -> Vec<Json> {
    texts.iter().map(|text| json(text)).collect()
}

pub(crate) fn agent(agent_id: &str, agent_name: &str) -> AgentResponse {
    let text = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_name}","agent_card_params":{{"model":"m"}}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn loaded(texts: &[&str]) -> crate::agent_registry::AgentRegistry {
    let registry = crate::agent_registry::AgentRegistry::default();
    let config = entries(texts);
    registry.load_agents_from_config(Some(&config)).unwrap();
    registry
}
