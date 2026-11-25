#![cfg(test)]

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::model_team_scope::postgres::PostgresScopeReads;
use crate::model_team_scope::reads::TeamScopeReads as _;

// The columns these reads touch, as LiteLLM's migrations declare them; the lists may be stored null.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    litellm_params JSONB NOT NULL,
    model_info JSONB,
    blocked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP,
    created_by TEXT,
    updated_at TIMESTAMP,
    updated_by TEXT
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    teams JSONB,
    models JSONB
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    models JSONB
);
"#;

const INSERT_MODEL_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, model_info) VALUES ($1, $2, ($3::text)::jsonb, ($4::text)::jsonb)"#;
const INSERT_USER_SQL: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams) VALUES ($1, ($2::text)::jsonb)"#;
const INSERT_TEAM_SQL: &str =
    r#"INSERT INTO "LiteLLM_TeamTable" (team_id, models) VALUES ($1, ($2::text)::jsonb)"#;
const DROP_MODEL_SQL: &str = r#"DROP TABLE "LiteLLM_ProxyModelTable""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    async fn start() -> Self {
        let url = Environment::parse_from(["model-team-scope-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .expect("the database fixture");
        let manager = Manager::new(
            fixture
                .options
                .url
                .expose_secret()
                .parse()
                .expect("the database url"),
            NoTls,
        );
        let pool = Pool::builder(manager)
            .max_size(2)
            .build()
            .expect("the database pool");
        Self {
            fixture,
            pool: pool.clone(),
            store: PostgresStore { pool: Some(pool) },
            availability: Availability::default(),
        }
    }

    const fn reads(&self) -> PostgresScopeReads<'_> {
        PostgresScopeReads {
            client: Some(PrismaClient {
                writer: &self.store,
                reader: &self.store,
                availability: &self.availability,
            }),
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn batch(&self, statements: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .batch_execute(statements)
            .await
            .expect("the statements run");
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

fn json_list(values: &[&str]) -> String {
    let items: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    format!("[{}]", items.join(","))
}

// A stored model row as the DB returns it: model_info and litellm_params are JSONB text here.
async fn seed_model(db: &Database, model_id: &str, model_name: &str, params: &str, info: &str) {
    db.execute(INSERT_MODEL_SQL, &[&model_id, &model_name, &params, &info])
        .await;
}

#[tokio::test]
async fn user_teams_read() {
    let db = Database::start().await;
    let teams = json_list(&["team-a", "team-b"]);
    db.execute(INSERT_USER_SQL, &[&"alice", &teams]).await;
    let reads = db.reads();
    assert_eq!(
        reads.user_teams("alice").await.unwrap(),
        Some(vec![String::from("team-a"), String::from("team-b")])
    );
    assert_eq!(reads.user_teams("missing").await.unwrap(), None);
    db.close().await;
}

#[tokio::test]
async fn team_models_read() {
    let db = Database::start().await;
    let models = json_list(&["gpt-4o"]);
    db.execute(INSERT_TEAM_SQL, &[&"team-a", &models]).await;
    let reads = db.reads();
    assert_eq!(
        reads.team_models("team-a").await.unwrap(),
        Some(vec![String::from("gpt-4o")])
    );
    assert_eq!(reads.team_models("missing").await.unwrap(), None);
    db.close().await;
}

#[tokio::test]
async fn model_ids_by_names() {
    let db = Database::start().await;
    seed_model(&db, "d1", "gpt-4o", r#"{"model":"openai/gpt-4o"}"#, "{}").await;
    seed_model(
        &db,
        "d2",
        "gpt-4o-mini",
        r#"{"model":"openai/gpt-4o-mini"}"#,
        "{}",
    )
    .await;
    let reads = db.reads();
    let names = vec![
        String::from("gpt-4o"),
        String::from("gpt-4o-mini"),
        String::from("gpt-4o"),
    ];
    let mut found = reads.model_ids_by_names(&names).await.unwrap();
    found.sort();
    assert_eq!(found, vec![String::from("d1"), String::from("d2")]);
    let missing = vec![String::from("zzz")];
    assert!(reads.model_ids_by_names(&missing).await.unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn model_row_keeps_stored_params() {
    let db = Database::start().await;
    seed_model(
        &db,
        "d1",
        "gpt-4o",
        r#"{"model":"openai/gpt-4o","api_key":"sk-plain"}"#,
        r#"{"id":"d1","team_id":"team-a"}"#,
    )
    .await;
    let reads = db.reads();
    let row = reads
        .model_row("d1")
        .await
        .unwrap()
        .expect("the stored row");
    assert_eq!(row.fields.model_name, "gpt-4o");
    assert_eq!(
        row.fields.litellm_params.0.get("api_key"),
        Some(&JsonData::String(String::from("sk-plain")))
    );
    assert!(reads.model_row("missing").await.unwrap().is_none());
    db.close().await;
}

#[tokio::test]
async fn missing_database_fails_reads() {
    let reads = PostgresScopeReads { client: None };
    reads.user_teams("alice").await.unwrap_err();
    reads.team_models("team-a").await.unwrap_err();
    let names = vec![String::from("gpt-4o")];
    reads.model_ids_by_names(&names).await.unwrap_err();
    reads.model_row("d1").await.unwrap_err();
}

#[tokio::test]
async fn dropped_model_table_fails_row_read() {
    let db = Database::start().await;
    db.batch(DROP_MODEL_SQL).await;
    db.reads().model_row("d1").await.unwrap_err();
    db.close().await;
}
