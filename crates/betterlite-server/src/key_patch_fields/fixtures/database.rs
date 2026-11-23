use std::collections::BTreeMap;

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

// Every column the key patch writes, plus the check that makes the second write of a transaction fail.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT NOT NULL,
    key_alias TEXT,
    spend DOUBLE PRECISION DEFAULT 0,
    models TEXT[] DEFAULT ARRAY[]::TEXT[],
    metadata JSONB DEFAULT '{}'::jsonb,
    model_max_budget JSONB DEFAULT '{}'::jsonb,
    router_settings JSONB,
    budget_limits JSONB,
    expires TIMESTAMP,
    budget_duration TEXT,
    budget_reset_at TIMESTAMP,
    budget_id TEXT,
    team_id TEXT,
    soft_budget_cooldown BOOLEAN DEFAULT FALSE,
    settings_updated_at TIMESTAMP,
    updated_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    object_permission_id TEXT,
    CONSTRAINT "LiteLLM_VerificationToken_pkey" PRIMARY KEY ("token"),
    CONSTRAINT "LiteLLM_VerificationToken_rejected" CHECK ("key_alias" IS DISTINCT FROM 'rejected')
);
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT NOT NULL,
    soft_budget DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_BudgetTable_pkey" PRIMARY KEY ("budget_id")
);
CREATE TABLE "LiteLLM_ObjectPermissionTable" (
    object_permission_id TEXT NOT NULL,
    mcp_servers TEXT[] DEFAULT ARRAY[]::TEXT[],
    mcp_access_groups TEXT[] DEFAULT ARRAY[]::TEXT[],
    mcp_tool_permissions JSONB,
    vector_stores TEXT[] DEFAULT ARRAY[]::TEXT[],
    agents TEXT[] DEFAULT ARRAY[]::TEXT[],
    agent_access_groups TEXT[] DEFAULT ARRAY[]::TEXT[],
    models TEXT[] DEFAULT ARRAY[]::TEXT[],
    mcp_toolsets TEXT[] DEFAULT ARRAY[]::TEXT[],
    blocked_tools TEXT[] DEFAULT ARRAY[]::TEXT[],
    search_tools TEXT[] DEFAULT ARRAY[]::TEXT[],
    mcp_tool_search_enabled BOOLEAN,
    skills TEXT[] DEFAULT ARRAY[]::TEXT[],
    CONSTRAINT "LiteLLM_ObjectPermissionTable_pkey" PRIMARY KEY ("object_permission_id")
);
"#;

const SEED_BUDGET_SQL: &str = r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, soft_budget, created_by, updated_by) VALUES ($1, $2, $3, $3)"#;
const SEED_KEY_SQL: &str = r#"INSERT INTO "LiteLLM_VerificationToken" (token, key_alias, metadata, budget_id, team_id) VALUES ($1, $2, $3::text::jsonb, $4, $5)"#;
const SEED_PERMISSION_SQL: &str =
    r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#;
const SELECT_BUDGET_SQL: &str =
    r#"SELECT row_to_json(t)::text FROM "LiteLLM_BudgetTable" AS t WHERE t."budget_id" = $1"#;
const SELECT_KEY_SQL: &str =
    r#"SELECT row_to_json(t)::text FROM "LiteLLM_VerificationToken" AS t WHERE t."token" = $1"#;
const SELECT_PERMISSION_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct TestDatabase {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    availability: Availability,
}

impl TestDatabase {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["b894-key-patch-fields-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .unwrap();
        let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
        let pool = Pool::builder(manager).max_size(4).build().unwrap();
        Self {
            fixture,
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            pool,
            availability: Availability::default(),
        }
    }

    pub(crate) const fn prisma(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(crate) async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.unwrap();
        client.execute(sql, params).await.unwrap();
    }

    pub(crate) async fn seed_budget(&self, budget_id: &str, soft_budget: Option<f64>) {
        self.execute(SEED_BUDGET_SQL, &[&budget_id, &soft_budget, &"seed"])
            .await;
    }

    pub(crate) async fn seed_key(
        &self,
        token: &str,
        key_alias: Option<&str>,
        metadata: &str,
        budget_id: Option<&str>,
        team_id: Option<&str>,
    ) {
        self.execute(
            SEED_KEY_SQL,
            &[&token, &key_alias, &metadata, &budget_id, &team_id],
        )
        .await;
    }

    pub(crate) async fn seed_permission(&self, object_permission_id: &str) {
        self.execute(SEED_PERMISSION_SQL, &[&object_permission_id])
            .await;
    }

    pub(crate) async fn budget_row(&self, budget_id: &str) -> Option<BTreeMap<String, JsonData>> {
        self.row(SELECT_BUDGET_SQL, budget_id).await
    }

    pub(crate) async fn key_row(&self, token: &str) -> Option<BTreeMap<String, JsonData>> {
        self.row(SELECT_KEY_SQL, token).await
    }

    pub(crate) async fn permission_count(&self) -> i64 {
        let client = self.pool.get().await.unwrap();
        client
            .query_one(SELECT_PERMISSION_SQL, &[])
            .await
            .unwrap()
            .get(0)
    }

    async fn row(&self, sql: &str, id: &str) -> Option<BTreeMap<String, JsonData>> {
        let client = self.pool.get().await.unwrap();
        let row = client.query_opt(sql, &[&id]).await.unwrap()?;
        let text: String = row.get(0);
        let JsonData::Object(fields) = serde_json::from_str(&text).unwrap() else {
            panic!("a stored row is a JSON object");
        };
        Some(fields)
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}
