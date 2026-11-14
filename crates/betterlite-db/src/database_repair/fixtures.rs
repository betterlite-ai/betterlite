#![cfg(test)]

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls, Row};

pub const SCHEMA_SQL: &str = include_str!("fixtures/schema.sql");

const SEED_TOKEN_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "team_id", "key_name") VALUES ($1::text, $2::text, $3::text)"#;

const SEED_SPEND_LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" ("request_id", "call_type", "api_key", "team_id", "startTime", "endTime") VALUES ($1::text, 'completion', $2::text, $3::text, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)"#;

const SPEND_TEAM_SQL: &str =
    r#"SELECT "team_id" FROM "LiteLLM_SpendLogs" WHERE "request_id" = $1::text"#;

const TOKEN_ROW_SQL: &str = r#"SELECT
    "key_name", "key_alias", "soft_budget_cooldown", "spend"::text AS spend, "expires", "models",
    "aliases"::text AS aliases, "config"::text AS config, "user_id", "team_id",
    "permissions"::text AS permissions, "max_parallel_requests", "metadata"::text AS metadata,
    "tpm_limit", "rpm_limit", "max_budget"::text AS max_budget, "budget_duration",
    "budget_reset_at", "allowed_cache_controls", "model_spend"::text AS model_spend,
    "model_max_budget"::text AS model_max_budget, "budget_id", "blocked", "created_at",
    "updated_at", "allowed_routes", "object_permission_id", "created_by", "updated_by",
    "organization_id"
FROM "LiteLLM_VerificationToken" WHERE "token" = $1::text"#;

const TOKEN_EXISTS_SQL: &str = r#"SELECT EXISTS (SELECT 1 FROM "LiteLLM_VerificationToken" WHERE "token" = $1::text) AS "present""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub struct RepairCase {
    pub fixture: PostgresFixture,
    pub client: Client,
    pub tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl RepairCase {
    pub async fn provision() -> Self {
        let url = Environment::parse_from(["database-repair"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_schema(url.as_ref())
            .resolve()
            .await
            .expect("a test schema");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
        client
            .batch_execute(SCHEMA_SQL)
            .await
            .expect("the pinned LiteLLM schema");
        Self {
            fixture,
            client,
            tasks,
        }
    }

    pub async fn close(self) {
        let Self {
            fixture,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        fixture.close_async().await.expect("the schema is dropped");
    }

    pub async fn seed_token(&self, token: &str, team_id: Option<&str>) {
        self.client
            .execute(
                SEED_TOKEN_SQL,
                &[&token, &team_id, &format!("seed-{token}")],
            )
            .await
            .expect("the seeded token");
    }

    pub async fn seed_spend_log(&self, request_id: &str, api_key: &str, team_id: Option<&str>) {
        self.client
            .execute(SEED_SPEND_LOG_SQL, &[&request_id, &api_key, &team_id])
            .await
            .expect("the seeded spend log");
    }

    pub async fn spend_team(&self, request_id: &str) -> Option<String> {
        self.client
            .query_one(SPEND_TEAM_SQL, &[&request_id])
            .await
            .expect("the spend log row")
            .get("team_id")
    }

    #[must_use]
    pub async fn token_row(&self, token: &str) -> Row {
        self.client
            .query_opt(TOKEN_ROW_SQL, &[&token])
            .await
            .expect("the token row query")
            .expect("the token row")
    }

    #[must_use]
    pub async fn token_present(&self, token: &str) -> bool {
        self.client
            .query_one(TOKEN_EXISTS_SQL, &[&token])
            .await
            .expect("the token lookup")
            .get("present")
    }
}
