use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::mcp::server_writes::store::PostgresServerWrites;

// Every column the write paths can name, so jsonb_populate_record has a target for each.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT PRIMARY KEY,
    server_name TEXT,
    alias TEXT,
    description TEXT,
    instructions TEXT,
    url TEXT,
    spec_path TEXT,
    transport TEXT NOT NULL DEFAULT 'sse',
    auth_type TEXT,
    credentials JSONB DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ DEFAULT now(),
    created_by TEXT,
    updated_at TIMESTAMPTZ DEFAULT now(),
    updated_by TEXT,
    mcp_info JSONB DEFAULT '{}'::jsonb,
    mcp_access_groups TEXT[] NOT NULL DEFAULT '{}',
    allowed_tools TEXT[] NOT NULL DEFAULT '{}',
    tool_name_to_display_name JSONB DEFAULT '{}'::jsonb,
    tool_name_to_description JSONB DEFAULT '{}'::jsonb,
    pinned_tools JSONB DEFAULT '{}'::jsonb,
    extra_headers TEXT[] NOT NULL DEFAULT '{}',
    static_headers JSONB DEFAULT '{}'::jsonb,
    env_vars JSONB DEFAULT '[]'::jsonb,
    status TEXT DEFAULT 'unknown',
    last_health_check TIMESTAMPTZ,
    health_check_error TEXT,
    command TEXT,
    args TEXT[] NOT NULL DEFAULT '{}',
    env JSONB DEFAULT '{}'::jsonb,
    issuer TEXT,
    authorization_url TEXT,
    token_url TEXT,
    registration_url TEXT,
    oauth2_flow TEXT,
    token_exchange_endpoint TEXT,
    audience TEXT,
    subject_token_type TEXT,
    token_exchange_profile TEXT,
    allow_all_keys BOOLEAN NOT NULL DEFAULT false,
    available_on_public_internet BOOLEAN NOT NULL DEFAULT true,
    delegate_auth_to_upstream BOOLEAN NOT NULL DEFAULT false,
    oauth_passthrough BOOLEAN NOT NULL DEFAULT false,
    dcr_bridge BOOLEAN,
    per_server_oauth_discovery BOOLEAN NOT NULL DEFAULT false,
    is_byok BOOLEAN NOT NULL DEFAULT false,
    byok_description TEXT[] NOT NULL DEFAULT '{}',
    byok_api_key_help_url TEXT,
    source_url TEXT,
    timeout DOUBLE PRECISION,
    max_concurrent_requests INTEGER,
    approval_status TEXT DEFAULT 'active',
    submitted_by TEXT,
    submitted_at TIMESTAMPTZ,
    reviewed_at TIMESTAMPTZ,
    review_notes TEXT
);
CREATE TABLE "LiteLLM_MCPUserCredentials" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    credential_b64 TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    values_b64 TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE (user_id, server_id)
);
CREATE TABLE "LiteLLM_MCPServerOAuthClient" (
    server_id TEXT PRIMARY KEY,
    credentials JSONB,
    created_at TIMESTAMPTZ DEFAULT now(),
    updated_at TIMESTAMPTZ DEFAULT now()
);
"#;

pub(crate) const SERVER_IDS_SQL: &str =
    r#"SELECT "server_id" FROM "LiteLLM_MCPServerTable" ORDER BY "server_id""#;
pub(crate) const SERVER_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_MCPServerTable""#;
pub(crate) const CHILD_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_MCPUserCredentials""#;
pub(crate) const ENV_VAR_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_MCPUserEnvVars""#;
pub(crate) const OAUTH_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_MCPServerOAuthClient""#;
pub(crate) const SEED_SERVER_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, transport, auth_type, credentials, updated_at) VALUES ($1, $2, 'sse', $3, to_jsonb($4::text), now())"#;
pub(crate) const SEED_DRAFT_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, server_name, transport, approval_status, env_vars, updated_at) VALUES ($1, $2, 'sse', 'draft', to_jsonb($3::text), now())"#;
pub(crate) const SERVER_NAME_COUNT_SQL: &str =
    r#"SELECT count(*) FROM "LiteLLM_MCPServerTable" WHERE "server_name" = $1"#;
pub(crate) const AGE_SERVER_SQL: &str = r#"UPDATE "LiteLLM_MCPServerTable" SET updated_at = now() - make_interval(secs => $2::double precision) WHERE "server_id" = $1"#;
const INSERT_USER_CREDENTIAL_SQL: &str = r#"INSERT INTO "LiteLLM_MCPUserCredentials" (id, user_id, server_id, credential_b64) VALUES ($1, $2, $3, 'AA')"#;
const INSERT_ENV_VAR_SQL: &str = r#"INSERT INTO "LiteLLM_MCPUserEnvVars" (id, user_id, server_id, values_b64) VALUES ($1, $2, $3, 'AA')"#;
const INSERT_OAUTH_SQL: &str = r#"INSERT INTO "LiteLLM_MCPServerOAuthClient" (server_id, credentials) VALUES ($1, '{}'::jsonb)"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Harness {
    fixture: PostgresFixture,
    pool: Pool,
    pub(crate) client: Client,
    writer: PostgresStore,
    availability: Availability,
    journal: Arc<Mutex<Vec<String>>>,
}

impl Harness {
    pub(crate) fn writes(&self) -> PostgresServerWrites<'_> {
        let journal = Arc::clone(&self.journal);
        let publish: PublishChange<TableError> = Arc::new(move |object_type: &str| {
            journal
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(object_type.to_owned());
            let future: PublishFuture<TableError> = Box::pin(async { Ok(()) });
            future
        });
        PostgresServerWrites {
            client: PrismaClient {
                writer: &self.writer,
                reader: &self.writer,
                availability: &self.availability,
            },
            publish,
        }
    }

    pub(crate) async fn raw_column(&self, sql: &str, param: &str) -> Option<String> {
        let row = self.client.query_opt(sql, &[&param]).await.unwrap()?;
        row.get(0)
    }

    pub(crate) async fn count(&self, sql: &str) -> i64 {
        self.client.query_one(sql, &[]).await.unwrap().get(0)
    }

    pub(crate) async fn count_param(&self, sql: &str, param: &str) -> i64 {
        self.client.query_one(sql, &[&param]).await.unwrap().get(0)
    }

    pub(crate) async fn server_ids(&self) -> Vec<String> {
        self.client
            .query(SERVER_IDS_SQL, &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    pub(crate) async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) {
        self.client.execute(sql, params).await.unwrap();
    }

    pub(crate) async fn seed_server(
        &self,
        server_id: &str,
        auth_type: Option<&str>,
        credentials: Option<&str>,
    ) {
        let server_name = format!("name-{server_id}");
        let params: [&(dyn ToSql + Sync); 4] = [&server_id, &server_name, &auth_type, &credentials];
        self.execute(SEED_SERVER_SQL, &params).await;
    }

    pub(crate) async fn seed_draft(&self, server_id: &str, age_seconds: f64) {
        let server_name = format!("name-{server_id}");
        self.execute(SEED_DRAFT_SQL, &[&server_id, &server_name, &"[]"])
            .await;
        self.age(server_id, age_seconds).await;
    }

    pub(crate) async fn age(&self, server_id: &str, age_seconds: f64) {
        self.execute(AGE_SERVER_SQL, &[&server_id, &age_seconds])
            .await;
    }

    pub(crate) async fn seed_children(&self, server_id: &str, users: &[&str]) {
        for user_id in users {
            let key = format!("{user_id}-{server_id}");
            let params: [&(dyn ToSql + Sync); 3] = [&key, user_id, &server_id];
            self.execute(INSERT_USER_CREDENTIAL_SQL, &params).await;
            let env_key = format!("env-{key}");
            let env_params: [&(dyn ToSql + Sync); 3] = [&env_key, user_id, &server_id];
            self.execute(INSERT_ENV_VAR_SQL, &env_params).await;
        }
        self.execute(INSERT_OAUTH_SQL, &[&server_id]).await;
    }

    pub(crate) async fn close(self) {
        let Self {
            fixture,
            pool,
            client,
            writer,
            ..
        } = self;
        drop(client);
        drop(writer);
        pool.close();
        fixture.close_async().await.unwrap();
    }
}

pub(crate) async fn database() -> Harness {
    let url = Environment::parse_from(["mcp-server-writes-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    let client = pool.get().await.unwrap();
    Harness {
        fixture,
        pool: pool.clone(),
        client,
        writer: PostgresStore { pool: Some(pool) },
        availability: Availability::default(),
        journal: Arc::new(Mutex::new(Vec::new())),
    }
}
