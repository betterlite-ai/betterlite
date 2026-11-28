use std::collections::BTreeMap;
use std::io;
use std::sync::PoisonError;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_model_patterns::view::{DeploymentView, ModelAccessView};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;
use tower::ServiceExt as _;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::auth::auth_identity::AuthContext;
pub(crate) use crate::auth::auth_key_lookup::tests::master_caller;
use crate::organization_create::creation::OrganizationCreateState;
use crate::organization_create::http::mount;

// Every column the creation, permission and member paths read, with the defaults the proxy schema gives them.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT NOT NULL,
    soft_budget DOUBLE PRECISION,
    max_budget DOUBLE PRECISION,
    max_parallel_requests BIGINT,
    tpm_limit BIGINT,
    rpm_limit BIGINT,
    tpd_limit BIGINT,
    model_max_budget JSONB,
    budget_duration TEXT,
    allowed_models TEXT[] DEFAULT ARRAY[]::TEXT[],
    temp_budget_increase DOUBLE PRECISION,
    temp_budget_expiry TIMESTAMP,
    created_by TEXT,
    updated_by TEXT,
    budget_reset_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_BudgetTable_pkey" PRIMARY KEY (budget_id)
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
    CONSTRAINT "LiteLLM_ObjectPermissionTable_pkey" PRIMARY KEY (object_permission_id)
);
CREATE TABLE "LiteLLM_OrganizationTable" (
    organization_id TEXT NOT NULL,
    organization_alias TEXT NOT NULL,
    budget_id TEXT NOT NULL REFERENCES "LiteLLM_BudgetTable"(budget_id),
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    metadata JSONB DEFAULT '{}'::jsonb,
    models TEXT[] DEFAULT ARRAY[]::TEXT[],
    model_spend JSONB DEFAULT '{}'::jsonb,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    object_permission_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_OrganizationTable_pkey" PRIMARY KEY (organization_id)
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT NOT NULL,
    user_email TEXT,
    user_role TEXT,
    models TEXT[] DEFAULT ARRAY[]::TEXT[],
    metadata JSONB,
    spend DOUBLE PRECISION DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_UserTable_pkey" PRIMARY KEY (user_id)
);
CREATE TABLE "LiteLLM_OrganizationMembership" (
    user_id TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    user_role TEXT,
    spend DOUBLE PRECISION DEFAULT 0.0,
    budget_id TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "LiteLLM_OrganizationMembership_pkey" PRIMARY KEY (user_id, organization_id)
);
CREATE TABLE "LiteLLM_MCPServerTable" (
    server_id TEXT NOT NULL,
    alias TEXT,
    server_name TEXT,
    CONSTRAINT "LiteLLM_MCPServerTable_pkey" PRIMARY KEY (server_id)
);
"#;

const SEED_USER_SQL: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, user_role, models) VALUES ($1, $2, $3)"#;
const SEED_MEMBERSHIP_SQL: &str = r#"INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, user_role) VALUES ($1, $2, $3)"#;
const SEED_BUDGET_SQL: &str = r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget, created_by, updated_by) VALUES ($1, $2, 'seed', 'seed')"#;
const SEED_ORGANIZATION_SQL: &str = r#"INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by) VALUES ($1, 'existing', 'existing-budget', 'seed', 'seed')"#;
const SEED_ORGANIZATION_BUDGET_SQL: &str = r#"INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('existing-budget', 'seed', 'seed')"#;
const SEED_MCP_SQL: &str =
    r#"INSERT INTO "LiteLLM_MCPServerTable" (server_id, alias, server_name) VALUES ($1, $2, $3)"#;
const DROP_PERMISSION_TABLE_SQL: &str = r#"DROP TABLE "LiteLLM_ObjectPermissionTable""#;
const ORGANIZATION_ROW_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_OrganizationTable" AS t WHERE t."organization_id" = $1"#;
const BUDGET_ROW_SQL: &str =
    r#"SELECT row_to_json(t)::text FROM "LiteLLM_BudgetTable" AS t WHERE t."budget_id" = $1"#;
const PERMISSION_ROW_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_ObjectPermissionTable" AS t WHERE t."object_permission_id" = $1"#;
const ORGANIZATION_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_OrganizationTable""#;
const BUDGET_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_BudgetTable""#;
const PERMISSION_COUNT_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_ObjectPermissionTable""#;
const DROP_USER_TABLE_SQL: &str = r#"DROP TABLE "LiteLLM_UserTable""#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct TestDatabase {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
}

impl TestDatabase {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["b826-organization-create-tests"])
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
        }
    }

    #[must_use]
    pub(crate) fn store(&self) -> Arc<PostgresStore> {
        Arc::new(self.store.clone())
    }

    pub(crate) async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.unwrap();
        client.execute(sql, params).await.unwrap();
    }

    pub(crate) async fn drop_user_table(&self) {
        self.execute(DROP_USER_TABLE_SQL, &[]).await;
    }

    pub(crate) async fn drop_permission_table(&self) {
        self.execute(DROP_PERMISSION_TABLE_SQL, &[]).await;
    }

    pub(crate) async fn seed_user(&self, user_id: &str, role: Option<&str>, models: &[&str]) {
        let model_list: Vec<&str> = models.to_vec();
        self.execute(SEED_USER_SQL, &[&user_id, &role, &model_list])
            .await;
    }

    pub(crate) async fn seed_membership(
        &self,
        user_id: &str,
        organization_id: &str,
        role: Option<&str>,
    ) {
        self.execute(SEED_MEMBERSHIP_SQL, &[&user_id, &organization_id, &role])
            .await;
    }

    pub(crate) async fn seed_organization(&self, organization_id: &str) {
        // the seeded row's budget reference must resolve, as the production foreign key requires
        self.execute(SEED_ORGANIZATION_BUDGET_SQL, &[]).await;
        self.execute(SEED_ORGANIZATION_SQL, &[&organization_id])
            .await;
    }

    pub(crate) async fn seed_budget(&self, budget_id: &str) {
        self.execute(SEED_BUDGET_SQL, &[&budget_id, &Some(25.0_f64)])
            .await;
    }

    pub(crate) async fn seed_mcp_server(&self, server_id: &str, alias: &str, name: &str) {
        self.execute(SEED_MCP_SQL, &[&server_id, &alias, &name])
            .await;
    }

    pub(crate) async fn organization_row(
        &self,
        organization_id: &str,
    ) -> Option<BTreeMap<String, JsonData>> {
        self.row(ORGANIZATION_ROW_SQL, organization_id).await
    }

    pub(crate) async fn budget_row(&self, budget_id: &str) -> Option<BTreeMap<String, JsonData>> {
        self.row(BUDGET_ROW_SQL, budget_id).await
    }

    pub(crate) async fn permission_row(
        &self,
        object_permission_id: &str,
    ) -> Option<BTreeMap<String, JsonData>> {
        self.row(PERMISSION_ROW_SQL, object_permission_id).await
    }

    pub(crate) async fn organization_count(&self) -> i64 {
        self.count(ORGANIZATION_COUNT_SQL).await
    }

    pub(crate) async fn budget_count(&self) -> i64 {
        self.count(BUDGET_COUNT_SQL).await
    }

    pub(crate) async fn permission_count(&self) -> i64 {
        self.count(PERMISSION_COUNT_SQL).await
    }

    pub(crate) async fn organization_ids(&self) -> Vec<String> {
        let client = self.pool.get().await.unwrap();
        let rows = client
            .query(r#"SELECT organization_id FROM "LiteLLM_OrganizationTable" ORDER BY organization_id"#, &[])
            .await
            .unwrap();
        rows.iter().map(|row| row.get(0)).collect()
    }

    async fn count(&self, sql: &str) -> i64 {
        let client = self.pool.get().await.unwrap();
        client.query_one(sql, &[]).await.unwrap().get(0)
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

pub(crate) struct TestApp {
    pub(crate) database: TestDatabase,
    pub(crate) state: Arc<OrganizationCreateState>,
}

impl TestApp {
    pub(crate) async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    pub(crate) async fn start_with<F>(configure: F) -> Self
    where
        F: FnOnce(&mut OrganizationCreateState),
    {
        let database = TestDatabase::start().await;
        let mut state = OrganizationCreateState {
            database: Some(database.store()),
            router_configured: true,
            view: Arc::new(DeploymentView::default()),
            max_recurse_depth: 5,
        };
        configure(&mut state);
        Self {
            database,
            state: Arc::new(state),
        }
    }

    pub(crate) async fn post(&self, caller: AuthContext, body: &str) -> Reply {
        // boxed so each call site's future stays small; the handler chain is far past clippy's limit
        Box::pin(self.post_optional(Some(caller), body)).await
    }

    pub(crate) async fn post_optional(&self, caller: Option<AuthContext>, body: &str) -> Reply {
        let mut router = mount(Arc::clone(&self.state));
        if let Some(context) = caller {
            router = router.layer(Extension(context));
        }
        let request = Request::builder()
            .method("POST")
            .uri("/organization/new")
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Reply {
            status,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    pub(crate) async fn close(self) {
        self.database.close().await;
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: String,
}

impl Reply {
    #[must_use]
    pub(crate) fn object(&self) -> BTreeMap<String, JsonData> {
        let JsonData::Object(fields) = serde_json::from_str(&self.body).unwrap() else {
            panic!(
                "the response body is a JSON object: {body}",
                body = self.body
            );
        };
        fields
    }

    #[must_use]
    pub(crate) fn field(&self, name: &str) -> JsonData {
        self.object()
            .remove(name)
            .unwrap_or_else(|| panic!("the response has no {name}: {body}", body = self.body))
    }

    #[must_use]
    pub(crate) fn text_field(&self, name: &str) -> String {
        let body = &self.body;
        let JsonData::String(text) = self.field(name) else {
            panic!("the response field {name} is not a string: {body}");
        };
        text
    }

    #[must_use]
    pub(crate) fn detail_error(&self) -> String {
        self.nested_text("detail", "error")
    }

    #[must_use]
    pub(crate) fn error_text(&self, name: &str) -> String {
        self.nested_text("error", name)
    }

    fn nested_text(&self, name: &str, field: &str) -> String {
        let JsonData::Object(object) = self.field(name) else {
            panic!(
                "the response field {name} is not an object: {body}",
                body = self.body
            );
        };
        match object.get(field) {
            Some(JsonData::String(text)) => text.clone(),
            _ => panic!(
                "the response field {name}.{field} is not a string: {body}",
                body = self.body
            ),
        }
    }
}

fn key_fields(
    user_id: Option<&str>,
    role: Option<&str>,
    models: &[&str],
) -> BTreeMap<String, JsonData> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(
        String::from("token"),
        JsonData::String(String::from("hashed-token")),
    );
    if let Some(id) = user_id {
        fields.insert(String::from("user_id"), JsonData::String(id.to_owned()));
    }
    if let Some(parsed_role) = role {
        fields.insert(
            String::from("user_role"),
            JsonData::String(parsed_role.to_owned()),
        );
    }
    fields.insert(
        String::from("models"),
        JsonData::Array(
            models
                .iter()
                .map(|model| JsonData::String((*model).to_owned()))
                .collect(),
        ),
    );
    fields
}

#[must_use]
pub(crate) fn key_caller(
    user_id: Option<&str>,
    role: Option<&str>,
    models: &[&str],
) -> AuthContext {
    let fields = key_fields(user_id, role, models);
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: String::from("key:hashed-token"),
            issuer: None,
            audience: Vec::new(),
            user: user_id.map(|id| UserIdentity {
                id: id.to_owned(),
                external_id: None,
                user_name: None,
                email: None,
                display_name: None,
            }),
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(Some(key)),
        },
    }
}

#[must_use]
pub(crate) fn admin_caller(user_id: Option<&str>) -> AuthContext {
    key_caller(user_id, Some("proxy_admin"), &[])
}

#[must_use]
pub(crate) fn role_only_caller(roles: &[Role]) -> AuthContext {
    jwt_principal(None, roles)
}

fn jwt_principal(user_id: Option<&str>, roles: &[Role]) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: user_id
                .map_or_else(|| String::from("jwt:anonymous"), |id| format!("jwt:{id}")),
            issuer: None,
            audience: Vec::new(),
            user: user_id.map(|id| UserIdentity {
                id: id.to_owned(),
                external_id: None,
                user_name: None,
                email: None,
                display_name: None,
            }),
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::BearerJwt,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(None),
        },
    }
}

#[derive(Debug, Default)]
pub(crate) struct RecordingView {
    pub(crate) calls: Mutex<Vec<String>>,
}

impl RecordingView {
    #[must_use]
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelAccessView for RecordingView {
    fn access_groups(&self, model: &str, _team_id: Option<&str>) -> BTreeMap<String, Vec<String>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        BTreeMap::new()
    }

    fn model_group_alias_target(&self, _model: &str) -> Option<String> {
        None
    }

    fn has_deployment(&self, _model_name: &str) -> bool {
        false
    }

    fn model_names(&self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    #[must_use]
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

#[must_use]
pub(crate) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[must_use]
pub(crate) fn parsed(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

#[must_use]
pub(crate) fn number(value: &JsonData) -> f64 {
    let JsonData::Number(number) = value else {
        panic!("the value is not a JSON number");
    };
    number.as_f64().unwrap()
}

// a JSON number equals the expected double: total_cmp keeps clippy's float comparison rule out of every call
pub(crate) fn assert_number(value: &JsonData, expected: f64) {
    let found = number(value);
    assert!(
        found.total_cmp(&expected).is_eq(),
        "expected {expected}, found {found}"
    );
}
