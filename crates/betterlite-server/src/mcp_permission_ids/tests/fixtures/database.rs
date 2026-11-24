use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ObjectPermissionTable" (
    object_permission_id TEXT NOT NULL,
    mcp_servers TEXT[] DEFAULT ARRAY[]::TEXT[],
    mcp_tool_permissions JSONB,
    CONSTRAINT "LiteLLM_ObjectPermissionTable_pkey" PRIMARY KEY ("object_permission_id")
);
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT
);
"#;

const SEED_PERMISSION_SQL: &str = r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers) VALUES ($1, $2)"#;
const SEED_JSON_PERMISSION_SQL: &str = r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers) VALUES ($1, $2::text::jsonb)"#;
const SEED_ID_ONLY_SQL: &str =
    r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ($1)"#;
const SEED_SERVER_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, alias) VALUES ($1, $2, $3)"#;
// Postgres cannot cast the column's array default during the type change, so it goes first
const ALTER_SERVERS_JSONB_SQL: &str = r#"
ALTER TABLE "LiteLLM_ObjectPermissionTable" ALTER COLUMN mcp_servers DROP DEFAULT;
ALTER TABLE "LiteLLM_ObjectPermissionTable" ALTER COLUMN mcp_servers TYPE JSONB USING to_jsonb(mcp_servers);
"#;
const DROP_SERVERS_COLUMN_SQL: &str =
    r#"ALTER TABLE "LiteLLM_ObjectPermissionTable" DROP COLUMN mcp_servers"#;
const DROP_PERMISSION_TABLE_SQL: &str = r#"DROP TABLE "LiteLLM_ObjectPermissionTable""#;

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
        let url = Environment::parse_from(["b817-mcp-permission-ids-tests"])
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

    pub(crate) async fn batch(&self, sql: &str) {
        let client = self.pool.get().await.unwrap();
        client.batch_execute(sql).await.unwrap();
    }

    pub(crate) async fn seed_permission_servers(&self, id: &str, servers: &(dyn ToSql + Sync)) {
        self.execute(SEED_PERMISSION_SQL, &[&id, servers]).await;
    }

    pub(crate) async fn seed_permission_json_servers(&self, id: &str, stored: &str) {
        self.execute(SEED_JSON_PERMISSION_SQL, &[&id, &stored])
            .await;
    }

    pub(crate) async fn seed_permission_without_servers(&self, id: &str) {
        self.execute(SEED_ID_ONLY_SQL, &[&id]).await;
    }

    pub(crate) async fn seed_server(&self, id: &str, name: Option<&str>, alias: Option<&str>) {
        self.execute(SEED_SERVER_SQL, &[&id, &name, &alias]).await;
    }

    pub(crate) async fn alter_servers_to_jsonb(&self) {
        self.batch(ALTER_SERVERS_JSONB_SQL).await;
    }

    pub(crate) async fn drop_servers_column(&self) {
        self.batch(DROP_SERVERS_COLUMN_SQL).await;
    }

    pub(crate) async fn drop_permission_table(&self) {
        self.batch(DROP_PERMISSION_TABLE_SQL).await;
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}
