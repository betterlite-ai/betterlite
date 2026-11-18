#![cfg(test)]

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::NaiveDateTime;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};

use super::{SpendPartitionError, SpendPartitionOperation, run_conversion};

const SPEND_LOGS_TABLE: &str = r#"CREATE TABLE "LiteLLM_SpendLogs" (
    "request_id" TEXT NOT NULL,
    "call_type" TEXT NOT NULL,
    "api_key" TEXT NOT NULL DEFAULT '',
    "spend" DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "total_tokens" INTEGER NOT NULL DEFAULT 0,
    "prompt_tokens" INTEGER NOT NULL DEFAULT 0,
    "completion_tokens" INTEGER NOT NULL DEFAULT 0,
    "startTime" TIMESTAMP(3) NOT NULL,
    "endTime" TIMESTAMP(3) NOT NULL,
    "completionStartTime" TIMESTAMP(3),
    "model" TEXT NOT NULL DEFAULT '',
    "model_id" TEXT DEFAULT '',
    "model_group" TEXT DEFAULT '',
    "custom_llm_provider" TEXT DEFAULT '',
    "api_base" TEXT DEFAULT '',
    "user" TEXT DEFAULT '',
    "metadata" JSONB DEFAULT '{}',
    "cache_hit" TEXT DEFAULT '',
    "cache_key" TEXT DEFAULT '',
    "request_tags" JSONB DEFAULT '[]',
    "team_id" TEXT,
    "end_user" TEXT,
    "requester_ip_address" TEXT,
    "messages" JSONB DEFAULT '{}',
    "response" JSONB DEFAULT '{}',
    "proxy_server_request" JSONB DEFAULT '{}',
    "session_id" TEXT,
    "status" TEXT,
    "mcp_namespaced_tool_name" TEXT,
    "organization_id" TEXT,
    "agent_id" TEXT,
    "request_duration_ms" INTEGER,
    "created_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "litellm_call_id" TEXT,
    "billing_agent_id" TEXT,
    CONSTRAINT "LiteLLM_SpendLogs_pkey" PRIMARY KEY ("request_id")
);

CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_startTime_idx" ON "LiteLLM_SpendLogs"("startTime");
CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_end_user_idx" ON "LiteLLM_SpendLogs"("end_user");
CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_session_id_idx" ON "LiteLLM_SpendLogs"("session_id");
CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_startTime_request_id_idx" ON "LiteLLM_SpendLogs"("startTime", "request_id");
CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_api_key_startTime_idx" ON "LiteLLM_SpendLogs"("api_key", "startTime");"#;

const LOG_SQL: &str = r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, "startTime", "endTime") VALUES ($1, 'completion', 'sk-fixture', $2, $2)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub struct SpendPartitionCase {
    pub(crate) fixture: PostgresFixture,
    pub(crate) client: Client,
    tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl SpendPartitionCase {
    pub async fn provision() -> Self {
        let url = Environment::parse_from(["spend-partitions"])
            .url
            .map(SecretString::from);
        let tables = vec![SPEND_LOGS_TABLE.to_owned()];
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), tables)
            .resolve()
            .await
            .expect("a test database");
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .expect("a test connection");
        let mut tasks = JoinSet::new();
        tasks.spawn(connection);
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
        fixture
            .close_async()
            .await
            .expect("the database is dropped");
    }

    pub async fn run_ok(&self, operation: SpendPartitionOperation) {
        run_conversion(&self.client, operation)
            .await
            .expect("the spend partition conversion");
    }

    #[must_use]
    pub async fn run_error(&self, operation: SpendPartitionOperation) -> SpendPartitionError {
        run_conversion(&self.client, operation)
            .await
            .expect_err("a failing spend partition conversion")
    }

    pub async fn seed_log(&self, request_id: &str, start_time: NaiveDateTime) {
        self.client
            .execute(LOG_SQL, &[&request_id, &start_time])
            .await
            .expect("a spend log row");
    }
}
