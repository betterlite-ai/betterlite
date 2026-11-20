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

// Every column the Prisma model declares, with its migration default, and the two rows the resolver reads.
const SCHEMA: &str = r#"
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
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT
);
CREATE TABLE "B816PermissionParents" (
    parent_id TEXT PRIMARY KEY,
    object_permission_id TEXT NOT NULL,
    CONSTRAINT "B816PermissionParents_checked" CHECK (parent_id <> 'bad')
);
"#;

const SELECT_PERMISSION_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_ObjectPermissionTable" AS t WHERE t."object_permission_id" = $1"#;
const COUNT_PERMISSIONS_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable""#;
const SEED_PERMISSION_SQL: &str = r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers, mcp_tool_permissions) VALUES ($1, ARRAY(SELECT jsonb_array_elements_text($2::jsonb)), to_jsonb($3::text))"#;
const SEED_SERVER_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, alias) VALUES ($1, $2, $3)"#;
const INSERT_PARENT_SQL: &str =
    r#"INSERT INTO "B816PermissionParents" (parent_id, object_permission_id) VALUES ($1, $2)"#;
const DROP_PERMISSIONS_SQL: &str = r#"DROP TABLE "LiteLLM_ObjectPermissionTable""#;

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
        let url = Environment::parse_from(["b816-object-permissions-tests"])
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

    // A stored grant is a JSON string, so the seed wraps the grant text into a jsonb string.
    pub(crate) async fn seed_permission(&self, id: &str, servers: &str, tools: Option<&str>) {
        self.execute(SEED_PERMISSION_SQL, &[&id, &servers, &tools])
            .await;
    }

    pub(crate) async fn seed_server(&self, id: &str, name: Option<&str>, alias: Option<&str>) {
        self.execute(SEED_SERVER_SQL, &[&id, &name, &alias]).await;
    }

    pub(crate) async fn permission_row(
        &self,
        object_permission_id: &str,
    ) -> Option<BTreeMap<String, JsonData>> {
        let client = self.pool.get().await.unwrap();
        let row = client
            .query_opt(SELECT_PERMISSION_SQL, &[&object_permission_id])
            .await
            .unwrap()?;
        let text: String = row.get(0);
        let JsonData::Object(fields) = serde_json::from_str(&text).unwrap() else {
            panic!("a permission row is a JSON object");
        };
        Some(fields)
    }

    pub(crate) async fn permission_count(&self) -> i64 {
        let client = self.pool.get().await.unwrap();
        client
            .query_one(COUNT_PERMISSIONS_SQL, &[])
            .await
            .unwrap()
            .get(0)
    }

    pub(crate) async fn insert_parent(
        &self,
        parent_id: &str,
        object_permission_id: &str,
    ) -> Result<(), tokio_postgres::Error> {
        let client = self.pool.get().await.unwrap();
        client
            .execute(INSERT_PARENT_SQL, &[&parent_id, &object_permission_id])
            .await
            .map(drop)
    }

    pub(crate) async fn drop_permission_table(&self) {
        let client = self.pool.get().await.unwrap();
        client.batch_execute(DROP_PERMISSIONS_SQL).await.unwrap();
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}
