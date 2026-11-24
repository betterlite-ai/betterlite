#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::health_check_rows::{HealthCheckIdSource, HealthCheckRow, HealthCheckRows};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_daily_usage::ADD_HEALTH_CHECK_TABLE;
use betterlite_db::schema_policy_resources::HEALTH_CHECK_LATEST_PER_MODEL;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use chrono::{DateTime, NaiveDateTime, TimeZone as _, Utc};
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysError};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::auth::auth_identity::AuthContext;
use crate::health_access_and_connections::visibility::{HealthRouter, LookupFailure};
use crate::health_cycle::ports::{ProbeFailure, ProbeRequest, ProbeResponse, ProviderHealth};
use crate::health_status_routes::error::HealthRouteError;
use crate::health_status_routes::health_status_routes;
use crate::health_status_routes::save::HealthProbeCall;
use crate::health_status_routes::state::{
    CycleCatalog, DetailVisibility, HealthCheckResults, HealthCheckSource, HealthCycleState,
    HealthResultsSource, HealthStatusState, RandomSource, SkipDisabledModels,
};
use crate::model_deprecation::clock::UtcClock;

const SCHEMA: &str = "health_status_routes_tests";

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(crate) fn deployment(name: &str, id: &str, model: &str) -> DeploymentSpec {
    deployment_with_info(name, id, vec![("model", text(model))], Vec::new())
}

pub(crate) fn deployment_with_info(
    name: &str,
    id: &str,
    params: Vec<(&str, Json)>,
    info: Vec<(&str, Json)>,
) -> DeploymentSpec {
    let mut model_info = object(info);
    model_info.insert("id".to_owned(), text(id));
    DeploymentSpec::new(
        name.to_owned(),
        object(params),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .expect("the fixture deployment is valid")
}

// A deployment row with no model_info at all, as an unvalidated config entry reaches the route
pub(crate) fn deployment_without_id(name: &str, model: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: object(vec![("model", text(model))]),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    }
}

pub(crate) struct FakeRouter {
    pub(crate) deployments: Vec<DeploymentSpec>,
    pub(crate) fail: bool,
}

pub(crate) fn router(deployments: Vec<DeploymentSpec>) -> Arc<dyn HealthRouter> {
    Arc::new(FakeRouter {
        deployments,
        fail: false,
    })
}

pub(crate) fn failing_router() -> Arc<dyn HealthRouter> {
    Arc::new(FakeRouter {
        deployments: Vec::new(),
        fail: true,
    })
}

fn team_allows(row: &DeploymentSpec, team_id: Option<&str>) -> bool {
    let Some(team) = team_id else {
        return true;
    };
    match row.model_info.get("team_id") {
        Some(Json::Text(owner)) => owner.as_str() == team,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => true,
    }
}

impl FakeRouter {
    fn guard(&self) -> Result<(), LookupFailure> {
        if self.fail {
            return Err(LookupFailure::Lookup(LookupError::InvalidRow { index: 0 }));
        }
        Ok(())
    }

    fn scoped(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.guard()?;
        Ok(self
            .deployments
            .iter()
            .filter(|row| model_name.is_none_or(|name| row.model_name == name))
            .filter(|row| team_allows(row, team_id))
            .cloned()
            .collect())
    }
}

impl HealthRouter for FakeRouter {
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        self.guard()?;
        let found = self
            .deployments
            .iter()
            .find(|row| row.id() == Some(model_id));
        Ok(found.cloned())
    }

    fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.scoped(model_name, team_id)
    }

    fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        let rows = self.scoped(None, team_id)?;
        Ok(rows.into_iter().map(|row| row.model_name).collect())
    }

    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        self.guard()?;
        Ok(AccessGroupMap::new())
    }
}

pub(crate) struct FakeProvider {
    failing: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl FakeProvider {
    pub(crate) const fn new() -> Self {
        Self {
            failing: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing(self, model: &str) -> Self {
        self.failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ProviderHealth for FakeProvider {
    async fn check(&self, request: ProbeRequest) -> Result<ProbeResponse, ProbeFailure> {
        let model = match request.params.get("model") {
            Some(Json::Text(value)) => value.clone(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => String::new(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.clone());
        let fails = self
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&model);
        if fails {
            return Err(ProbeFailure {
                message: "the provider failed".to_owned(),
                status_code: 400,
                raw_request_typed_dict: None,
            });
        }
        Ok(ProbeResponse {
            remaining_requests: Some("42".to_owned()),
            remaining_tokens: None,
            region: None,
        })
    }
}

#[derive(Default)]
pub(crate) struct FakeResults {
    pub(crate) snapshot: HealthCheckResults,
}

impl HealthResultsSource for FakeResults {
    fn snapshot(&self) -> HealthCheckResults {
        self.snapshot.clone()
    }
}

struct SeededRandom(u64);

impl RandomSource for SeededRandom {
    fn generator(&self) -> Result<StdRng, SysError> {
        Ok(StdRng::seed_from_u64(self.0))
    }
}

// A source that refuses to seed, as the operating system's entropy call can
pub(crate) struct FailingRandom;

impl RandomSource for FailingRandom {
    fn generator(&self) -> Result<StdRng, SysError> {
        Err(SysError::new_custom(1_u16))
    }
}

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 15, 10, 0, 0)
        .single()
        .expect("the fixed instant")
}

pub(crate) fn instant(value: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .expect("the calendar instant")
        .and_utc()
}

pub(crate) fn instant_with_fraction(value: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .expect("the fractional instant")
        .and_utc()
}

pub(crate) fn row(id: &str, name: &str, checked_at: DateTime<Utc>) -> HealthCheckRow {
    HealthCheckRow {
        health_check_id: id.to_owned(),
        model_name: name.to_owned(),
        model_id: Some("id-a".to_owned()),
        status: "healthy".to_owned(),
        healthy_count: 2,
        unhealthy_count: 1,
        error_message: None,
        response_time_ms: Some(12.5),
        details: None,
        checked_by: Some("user-1".to_owned()),
        checked_at,
        created_at: checked_at,
        updated_at: checked_at,
    }
}

pub(crate) fn text_field(object: &JsonObject, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn endpoint(model_name: &str, model_id: &str) -> JsonObject {
    object(vec![
        ("model_name", text(model_name)),
        ("model_id", text(model_id)),
        ("api_base", text("https://internal.example.com")),
    ])
}

pub(crate) const fn results(
    healthy: Vec<JsonObject>,
    unhealthy: Vec<JsonObject>,
) -> HealthCheckResults {
    HealthCheckResults {
        healthy_count: healthy.len(),
        unhealthy_count: unhealthy.len(),
        healthy_endpoints: healthy,
        unhealthy_endpoints: unhealthy,
    }
}

// One live probe over the given model list, named by nobody: the shape /health passes on the router path
pub(crate) fn probe(model_list: &[DeploymentSpec]) -> HealthProbeCall<'_> {
    HealthProbeCall {
        model_list,
        target_model: None,
        cli_model: None,
        model_id: None,
        team_id: None,
        user_id: Some("user-1"),
        start_time: instant("2026-03-15 09:59:00"),
    }
}

// The background cache the route reads, filled with these healthy endpoints
pub(crate) fn background(state: &mut HealthStatusState, healthy: Vec<JsonObject>) {
    let source = Arc::new(FakeResults {
        snapshot: results(healthy, Vec::new()),
    });
    state.results = Arc::<FakeResults>::clone(&source);
    state.use_background_health_checks = HealthCheckSource::Background;
}

// The deployment list and the router over it, as the server's state carries them
pub(crate) fn serve(state: &mut HealthStatusState, deployments: Vec<DeploymentSpec>) {
    state.router = Some(router(deployments.clone()));
    state.deployments = Some(Arc::new(deployments));
}

// The router catalog a cycle reads for dependency probes, indexed by model name and id
pub(crate) fn cycle_catalog(deployments: &[DeploymentSpec]) -> CycleCatalog {
    let mut catalog = DeploymentCatalog::default();
    for row in deployments {
        catalog.push(row.clone());
    }
    CycleCatalog {
        handle: Arc::new(CatalogHandle::from_pointee(catalog)),
        aliases: Arc::new(AliasMap::new()),
        routing_groups: Arc::new(RoutingGroupIndex::default()),
        inferred_provider: Arc::new(|_: &str| None),
    }
}

// Hand the pooled client back, then drain the tracker so a handed-off save reaches the table
pub(crate) async fn settle(database: &mut Database, tasks: &TaskTracker) {
    database.release();
    tasks.close();
    tasks.wait().await;
}

pub(crate) fn first_issue(body: &JsonObject) -> JsonObject {
    match body.get("detail") {
        Some(Json::List(items)) => match &items[0] {
            Json::Object(fields) => fields.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                panic!("the issue is an object")
            }
        },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => panic!("the body carries an issue list"),
    }
}

pub(crate) fn assert_issue(reply: &Reply, uri: &str, kind: &str, field: &str, message: &str) {
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
    let issue = first_issue(&reply.body);
    assert_eq!(text_field(&issue, "type"), Some(kind.to_owned()), "{uri}");
    assert_eq!(text_field(&issue, "msg"), Some(message.to_owned()), "{uri}");
    assert_eq!(
        issue.get("loc"),
        Some(&Json::List(vec![text("query"), text(field)])),
        "{uri}"
    );
}

pub(crate) fn latest_checks(reply: &Reply) -> JsonObject {
    match reply.body.get("latest_health_checks") {
        Some(Json::Object(map)) => map.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => panic!("latest_health_checks is an object"),
    }
}

pub(crate) fn fixture() -> (HealthStatusState, Arc<FakeProvider>, Arc<FakeResults>) {
    let provider = Arc::new(FakeProvider::new());
    let source = Arc::new(FakeResults::default());
    let mut bundled = BundledCatalog::default();
    let models = bundled.get();
    let state = HealthStatusState {
        deployments: None,
        router: None,
        user_model: None,
        health_check_details: DetailVisibility::Shown,
        health_check_concurrency: None,
        use_background_health_checks: HealthCheckSource::Live,
        skip_disabled_models: SkipDisabledModels::Keep,
        results: Arc::<FakeResults>::clone(&source),
        rows: None,
        clock: Arc::new(FixedClock(fixed_now())),
        tasks: TaskTracker::new(),
        cycle: HealthCycleState {
            provider: Arc::<FakeProvider>::clone(&provider),
            models,
            policy: ProbePolicy::default(),
            catalog: None,
            random: Arc::new(SeededRandom(7_u64)),
        },
    };
    (state, provider, source)
}

// A state whose store holds these rows, and the database the caller finishes
pub(crate) async fn state_with_rows(rows: &[SeedRow]) -> (HealthStatusState, Database) {
    let mut database = database().await;
    insert_rows(&database, rows).await;
    database.release();
    let (mut state, _provider, _source) = fixture();
    state.rows = Some(Arc::new(database.rows()));
    (state, database)
}

pub(crate) fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

pub(crate) fn key_context(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fixture parses");
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture builds");
    context("key-owner", &[], SourceKey(Some(key)))
}

pub(crate) fn admin() -> AuthContext {
    key_context(r#"{"user_id":"admin-1","user_role":"proxy_admin"}"#)
}

// A key whose grants name some model names, or every model when the list is empty
pub(crate) fn user(models: &str) -> AuthContext {
    key_context(&format!(r#"{{"user_id":"user-1","models":{models}}}"#))
}

pub(crate) fn team_user(models: &str, team_id: &str) -> AuthContext {
    key_context(&format!(
        r#"{{"user_id":"user-1","models":{models},"team_id":"{team_id}"}}"#
    ))
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: JsonObject,
}

impl Reply {
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|value| value.to_str().expect("a text header").to_owned())
    }

    pub(crate) fn count(&self, key: &str) -> i64 {
        match self.body.get(key) {
            Some(Json::Number(number)) => number.as_i64().expect("an integer field"),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => panic!("the body has no integer {key}"),
        }
    }

    pub(crate) fn list(&self, key: &str) -> Vec<JsonObject> {
        match self.body.get(key) {
            Some(Json::List(rows)) => rows
                .iter()
                .map(|row| match row {
                    Json::Object(fields) => fields.clone(),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_) => {
                        panic!("the {key} rows are objects")
                    }
                })
                .collect(),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            )
            | None => panic!("the body has no list {key}"),
        }
    }

    pub(crate) fn detail_error(&self) -> String {
        detail_error(&self.body)
    }
}

pub(crate) fn detail_error(body: &JsonObject) -> String {
    let detail = body.get("detail").expect("the body carries a detail");
    match detail {
        Json::Object(fields) => match fields.get("error") {
            Some(Json::Text(message)) => message.clone(),
            _ => panic!("the detail carries an error"),
        },
        Json::Text(message) => message.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            panic!("the detail is text or an object")
        }
    }
}

pub(crate) async fn get(
    state: HealthStatusState,
    caller: Option<&AuthContext>,
    uri: &str,
) -> Reply {
    let application = health_status_routes(state);
    let mut request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds");
    if let Some(identity) = caller {
        request.extensions_mut().insert(identity.clone());
    }
    let response = application
        .oneshot(request)
        .await
        .expect("the route answers");
    read(response).await
}

async fn read(response: Response) -> Reply {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).expect("the body is a JSON object"),
    }
}

pub(crate) async fn render(error: HealthRouteError) -> Reply {
    read(error.into_response()).await
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn configured_url() -> Option<SecretString> {
    Environment::parse_from(["health-status-routes-tests"])
        .url
        .map(SecretString::from)
}

fn pool(url: &SecretString) -> Pool {
    let mut config: tokio_postgres::Config = url.expose_secret().parse().expect("the URL parses");
    config.options(format!("-csearch_path={SCHEMA}"));
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager)
        .max_size(1)
        .build()
        .expect("the postgres pool")
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    store: PostgresStore,
    client: Option<Client>,
}

pub(crate) async fn database() -> Database {
    let statements = vec![
        format!("CREATE SCHEMA IF NOT EXISTS \"{SCHEMA}\""),
        format!("SET search_path TO \"{SCHEMA}\""),
        str::from_utf8(ADD_HEALTH_CHECK_TABLE.bytes)
            .expect("the table migration is UTF-8")
            .to_owned(),
        str::from_utf8(HEALTH_CHECK_LATEST_PER_MODEL.bytes)
            .expect("the index migration is UTF-8")
            .to_owned(),
    ];
    let fixture = PostgresFixture::provision_with_statements(configured_url().as_ref(), statements)
        .resolve()
        .await
        .expect("the postgres fixture");
    let store = PostgresStore {
        pool: Some(pool(&fixture.options.url)),
    };
    let client = store.session().await.expect("the held client");
    Database {
        fixture,
        store,
        client: Some(client),
    }
}

impl Database {
    pub(crate) fn rows(&self) -> HealthCheckRows {
        HealthCheckRows {
            store: self.store.clone(),
            new_id: Arc::new(|| uuid::Uuid::new_v4().to_string()),
        }
    }

    // Returns the pool's only client, so a task waiting for a connection can run
    pub(crate) fn release(&mut self) {
        self.client.take();
    }

    pub(crate) async fn finish(self) {
        drop(self.client);
        if let Some(pool) = self.store.pool {
            pool.close();
        }
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

// A store with no pool: every call reaches the driver's "No DB Connected" error without a database
pub(crate) fn unconnected() -> HealthCheckRows {
    let source: HealthCheckIdSource = Arc::new(|| String::from("unused-id"));
    HealthCheckRows {
        store: PostgresStore { pool: None },
        new_id: source,
    }
}

#[derive(Clone)]
pub(crate) struct Seed {
    pub(crate) id: &'static str,
    pub(crate) model_name: &'static str,
    pub(crate) model_id: Option<&'static str>,
    pub(crate) status: &'static str,
    pub(crate) checked_at: &'static str,
}

impl Seed {
    pub(crate) const fn new(
        id: &'static str,
        model_name: &'static str,
        checked_at: &'static str,
    ) -> Self {
        Self {
            id,
            model_name,
            model_id: None,
            status: "healthy",
            checked_at,
        }
    }
}

// A seed row as a tuple: id, model_name, model_id, status, checked_at
pub(crate) type SeedRow = (
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
);

pub(crate) async fn insert_rows(database: &Database, rows: &[SeedRow]) {
    for &(id, model_name, model_id, status, checked_at) in rows {
        let mut seed = Seed::new(id, model_name, checked_at);
        seed.model_id = model_id;
        seed.status = status;
        insert_seed(stored_client(database), &seed).await;
    }
}

pub(crate) async fn insert_seed(client: &Client, seed: &Seed) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_HealthCheckTable"
                ("health_check_id", "model_name", "model_id", "status", "healthy_count",
                 "unhealthy_count", "checked_at", "created_at", "updated_at")
                VALUES ($1, $2, $3, $4, 2, 0, $5::text::timestamp, $5::text::timestamp, $5::text::timestamp)"#,
            &[
                &seed.id,
                &seed.model_name,
                &seed.model_id,
                &seed.status,
                &seed.checked_at,
            ],
        )
        .await
        .expect("the seed row");
}

pub(crate) fn stored_client(database: &Database) -> &Client {
    database.client.as_ref().expect("the held client")
}
