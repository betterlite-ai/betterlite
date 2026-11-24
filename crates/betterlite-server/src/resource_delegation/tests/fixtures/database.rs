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
    CONSTRAINT "LiteLLM_ObjectPermissionTable_pkey" PRIMARY KEY ("object_permission_id")
);
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT
);
"#;

const SEED_PERMISSION_SQL: &str = r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, mcp_servers) VALUES ($1, $2)"#;
const SEED_SERVER_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, alias) VALUES ($1, $2, $3)"#;

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
        let url = Environment::parse_from(["b818-resource-delegation-tests"])
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

    pub(crate) async fn seed_permission_servers(&self, id: &str, servers: &(dyn ToSql + Sync)) {
        let client = self.pool.get().await.unwrap();
        client
            .execute(SEED_PERMISSION_SQL, &[&id, servers])
            .await
            .unwrap();
    }

    pub(crate) async fn seed_server(&self, id: &str, name: Option<&str>, alias: Option<&str>) {
        let client = self.pool.get().await.unwrap();
        client
            .execute(SEED_SERVER_SQL, &[&id, &name, &alias])
            .await
            .unwrap();
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}
