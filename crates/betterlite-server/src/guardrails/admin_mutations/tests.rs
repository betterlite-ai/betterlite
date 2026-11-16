#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableBinding, TableError};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_guardrails::guardrail_registry::build::GuardrailFactories;
use betterlite_guardrails::guardrail_registry::handler::{
    HandlerSettings, InMemoryGuardrailHandler,
};
use betterlite_guardrails::guardrail_registry::registry::GuardrailRegistry;
use betterlite_guardrails::guardrail_registry::rows::{
    GuardrailRow, GuardrailRows, GuardrailUpdate, NewGuardrailRow, RowsError,
};
use betterlite_guardrails::guardrail_startup::runtime::GuardrailRuntime;
use betterlite_guardrails::tool_policy::{RequestToolNames, ToolPolicyLookup};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tower::ServiceExt as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::auth::auth_identity::AuthContext;

use super::body::{BodyError, WriteRequest, decode_patch_body, decode_write_request};
use super::review_rows::{
    PostgresReviewRows, ReviewRow, ReviewRows, ReviewRowsBinding, ReviewRowsError,
};
use super::{MutationDatabase, MutationState, routes};

include!("tests/body.rs");
include!("tests/write.rs");
include!("tests/review.rs");
include!("tests/storage.rs");

pub(super) const BEDROCK_PARAMS: &str = r#"{"guardrail":"bedrock","mode":"pre_call"}"#;
pub(super) const CREATE_JSON: &str = r#"{"guardrail":{"guardrail_name":"my-guard","litellm_params":{"guardrail":"bedrock","mode":"pre_call"}}}"#;
pub(super) const CREATE_WITH_INFO: &str = r#"{"guardrail":{"guardrail_name":"my-guard","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"guardrail_info":{"description":"guard"}}}"#;
pub(super) const CREATE_UNSUPPORTED: &str = r#"{"guardrail":{"guardrail_name":"my-guard","litellm_params":{"guardrail":"made-up","mode":"pre_call"}}}"#;
pub(super) const CREATE_ENTERPRISE: &str = r#"{"guardrail":{"guardrail_name":"my-guard","litellm_params":{"guardrail":"hide-secrets","mode":"pre_call"}}}"#;

pub(super) struct Harness {
    pub(super) router: Router,
    pub(super) guardrails: FakeGuardrails,
    pub(super) reviews: FakeReviews,
    pub(super) runtime: Arc<GuardrailRuntime<FakeGuardrails>>,
}

#[derive(Default)]
struct GuardrailStore {
    rows: BTreeMap<String, GuardrailRow>,
    created: usize,
    fail_create: bool,
    fail_delete: bool,
    fail_update: bool,
    fail_find: bool,
}

#[derive(Clone, Default)]
pub(super) struct FakeGuardrails(Arc<Mutex<GuardrailStore>>);

#[derive(Default)]
struct ReviewStore {
    rows: BTreeMap<String, ReviewRow>,
    statuses: Vec<String>,
    fail_set: bool,
    fail_find: bool,
}

#[derive(Clone, Default)]
pub(super) struct FakeReviews(Arc<Mutex<ReviewStore>>);

struct NoPolicies;

impl ToolPolicyLookup for NoPolicies {
    fn is_initialized(&self) -> bool {
        false
    }

    fn effective_policies(
        &self,
        _tool_names: &[String],
        _key_permission_id: Option<&str>,
        _team_permission_id: Option<&str>,
    ) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn output_policy(&self, _tool_name: &str) -> String {
        String::new()
    }
}

struct NoNames;

impl RequestToolNames for NoNames {
    fn names(&self, _route: &str, _request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

fn handler() -> InMemoryGuardrailHandler {
    let factories = GuardrailFactories::new(Arc::new(NoPolicies), Arc::new(NoNames));
    let settings = HandlerSettings {
        strict_modes: true,
        global_skip_tool_message: false,
    };
    InMemoryGuardrailHandler::new(factories, Box::new(BTreeMap::new()), settings)
}

fn harness() -> Harness {
    harness_with(FakeGuardrails::default(), FakeReviews::default())
}

fn harness_with(guardrails: FakeGuardrails, reviews: FakeReviews) -> Harness {
    let runtime = Arc::new(GuardrailRuntime::new(
        handler(),
        GuardrailRegistry::new(guardrails.clone()),
    ));
    let state = MutationState {
        database: Some(MutationDatabase {
            runtime: Arc::clone(&runtime),
            review_rows: Arc::new(reviews.clone()),
        }),
    };
    Harness {
        router: routes(state),
        guardrails,
        reviews,
        runtime,
    }
}

fn bare_router() -> Router {
    let state: MutationState<FakeGuardrails, FakeReviews> = MutationState { database: None };
    routes(state)
}

pub(super) fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

fn admin() -> AuthContext {
    caller("master_key", Vec::new())
}

fn role_admin() -> AuthContext {
    caller("admin-user", vec![Role::PlatformAdmin])
}

fn viewer() -> AuthContext {
    caller("viewer-user", vec![Role::PlatformViewer])
}

fn request(
    method: &str,
    uri: &str,
    auth: Option<AuthContext>,
    body: Option<&str>,
) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    let builder = match auth {
        Some(context) => builder.extension(context),
        None => builder,
    };
    let payload = body.unwrap_or_default().to_owned();
    builder.body(Body::from(payload)).expect("a request")
}

pub(super) async fn send(application: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = application
        .oneshot(request)
        .await
        .expect("the route answers");
    body_of(response).await
}

async fn body_of(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the body is UTF-8"),
    )
}

pub(super) fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

pub(super) fn field<'a>(value: &'a JsonData, name: &str) -> &'a JsonData {
    match value {
        JsonData::Object(fields) => fields
            .get(name)
            .unwrap_or_else(|| panic!("the body has no {name}: {value:?}")),
        other => panic!("not an object: {other:?}"),
    }
}

pub(super) fn keys(value: &JsonData) -> Vec<String> {
    match value {
        JsonData::Object(fields) => fields.keys().cloned().collect(),
        other => panic!("not an object: {other:?}"),
    }
}

pub(super) fn text(value: &JsonData) -> &str {
    match value {
        JsonData::String(text) => text,
        other => panic!("not a string: {other:?}"),
    }
}

pub(super) fn detail(body: &str) -> String {
    text(field(&parse(body), "detail")).to_owned()
}

/// The first entry of a 422 body's `detail` list.
pub(super) fn first_issue(body: &str) -> JsonData {
    match field(&parse(body), "detail") {
        JsonData::Array(items) => items.first().cloned().expect("one issue"),
        other => panic!("the detail is not a list: {other:?}"),
    }
}

pub(super) fn stamp(text: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").expect("a stamp"))
}

pub(super) fn stored_row(id: &str, name: &str, params: &str, status: &str) -> GuardrailRow {
    GuardrailRow {
        guardrail_id: id.to_owned(),
        guardrail_name: name.to_owned(),
        litellm_params: Some(params.to_owned()),
        guardrail_info: Some("{}".to_owned()),
        team_id: None,
        status: status.to_owned(),
        created_at: stamp("2026-01-02T03:04:05"),
        updated_at: stamp("2026-01-02T03:04:05"),
    }
}

pub(super) fn review_row(id: &str, name: &str, params: Option<&str>, status: &str) -> ReviewRow {
    ReviewRow {
        guardrail_id: id.to_owned(),
        guardrail_name: name.to_owned(),
        litellm_params: params.map(parse),
        guardrail_info: None,
        team_id: None,
        status: status.to_owned(),
        submitted_at: Some(stamp("2026-01-02T03:04:05")),
        reviewed_at: None,
    }
}

impl FakeGuardrails {
    pub(super) fn insert(&self, row: GuardrailRow) {
        let mut store = self.0.lock().unwrap();
        store.rows.insert(row.guardrail_id.clone(), row);
    }

    pub(super) fn row(&self, id: &str) -> Option<GuardrailRow> {
        self.0.lock().unwrap().rows.get(id).cloned()
    }

    pub(super) fn rows(&self) -> Vec<GuardrailRow> {
        self.0.lock().unwrap().rows.values().cloned().collect()
    }

    pub(super) fn created(&self) -> usize {
        self.0.lock().unwrap().created
    }

    fn fail_create(&self) {
        self.0.lock().unwrap().fail_create = true;
    }

    fn fail_delete(&self) {
        self.0.lock().unwrap().fail_delete = true;
    }

    fn fail_update(&self) {
        self.0.lock().unwrap().fail_update = true;
    }

    fn fail_find(&self) {
        self.0.lock().unwrap().fail_find = true;
    }
}

impl GuardrailRows for FakeGuardrails {
    async fn create(&self, row: NewGuardrailRow) -> Result<GuardrailRow, RowsError> {
        let mut store = self.0.lock().unwrap();
        if store.fail_create {
            return Err(RowsError::MissingRow);
        }
        store.created += 1;
        let created = GuardrailRow {
            guardrail_id: row.guardrail_id,
            guardrail_name: row.fields.guardrail_name,
            litellm_params: Some(row.fields.litellm_params),
            guardrail_info: Some(row.fields.guardrail_info),
            team_id: None,
            status: "active".to_owned(),
            created_at: Timestamp::Aware(row.created_at.fixed_offset()),
            updated_at: Timestamp::Aware(row.fields.updated_at.fixed_offset()),
        };
        store
            .rows
            .insert(created.guardrail_id.clone(), created.clone());
        Ok(created)
    }

    async fn delete(&self, guardrail_id: &str) -> Result<(), RowsError> {
        let mut store = self.0.lock().unwrap();
        if store.fail_delete || store.rows.remove(guardrail_id).is_none() {
            return Err(RowsError::MissingRow);
        }
        Ok(())
    }

    async fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> Result<Option<GuardrailRow>, RowsError> {
        let mut store = self.0.lock().unwrap();
        if store.fail_update {
            return Err(RowsError::MissingRow);
        }
        let Some(existing) = store.rows.get_mut(guardrail_id) else {
            return Ok(None);
        };
        existing.guardrail_name = row.guardrail_name;
        existing.litellm_params = Some(row.litellm_params);
        existing.guardrail_info = Some(row.guardrail_info);
        existing.updated_at = Timestamp::Aware(row.updated_at.fixed_offset());
        Ok(Some(existing.clone()))
    }

    async fn find_active(&self) -> Result<Vec<GuardrailRow>, RowsError> {
        Ok(self
            .rows()
            .into_iter()
            .filter(|row| row.status == "active")
            .collect())
    }

    async fn find_by_id(&self, guardrail_id: &str) -> Result<Option<GuardrailRow>, RowsError> {
        if self.0.lock().unwrap().fail_find {
            return Err(RowsError::MissingRow);
        }
        Ok(self.row(guardrail_id))
    }

    async fn find_by_name(&self, guardrail_name: &str) -> Result<Option<GuardrailRow>, RowsError> {
        Ok(self
            .rows()
            .into_iter()
            .find(|row| row.guardrail_name == guardrail_name))
    }
}

impl FakeReviews {
    pub(super) fn insert(&self, row: ReviewRow) {
        let mut store = self.0.lock().unwrap();
        store.rows.insert(row.guardrail_id.clone(), row);
    }

    pub(super) fn row(&self, id: &str) -> Option<ReviewRow> {
        self.0.lock().unwrap().rows.get(id).cloned()
    }

    pub(super) fn statuses(&self) -> Vec<String> {
        self.0.lock().unwrap().statuses.clone()
    }

    fn fail_set(&self) {
        self.0.lock().unwrap().fail_set = true;
    }

    fn fail_find(&self) {
        self.0.lock().unwrap().fail_find = true;
    }
}

impl ReviewRows for FakeReviews {
    async fn find_unique(&self, guardrail_id: &str) -> Result<Option<ReviewRow>, ReviewRowsError> {
        let store = self.0.lock().unwrap();
        if store.fail_find {
            return Err(ReviewRowsError::MissingRow {
                guardrail_id: guardrail_id.to_owned(),
            });
        }
        Ok(store.rows.get(guardrail_id).cloned())
    }

    async fn set_status(
        &self,
        guardrail_id: &str,
        status: &str,
        now: DateTime<Utc>,
    ) -> Result<(), ReviewRowsError> {
        let mut store = self.0.lock().unwrap();
        if store.fail_set {
            return Err(ReviewRowsError::MissingRow {
                guardrail_id: guardrail_id.to_owned(),
            });
        }
        store.statuses.push(status.to_owned());
        if let Some(row) = store.rows.get_mut(guardrail_id) {
            row.status = status.to_owned();
            row.reviewed_at = Some(Timestamp::Aware(now.fixed_offset()));
        }
        Ok(())
    }
}

pub(super) async fn held_guardrail(
    runtime: &GuardrailRuntime<FakeGuardrails>,
    id: &str,
) -> Option<Guardrail> {
    let handler = runtime.handler().read().await;
    handler.get_guardrail_by_id(id).cloned()
}

pub(super) async fn held_source(
    runtime: &GuardrailRuntime<FakeGuardrails>,
    id: &str,
) -> Option<GuardrailDefinitionLocation> {
    let handler = runtime.handler().read().await;
    handler.get_source(id)
}

pub(super) async fn held_count(runtime: &GuardrailRuntime<FakeGuardrails>) -> usize {
    let handler = runtime.handler().read().await;
    handler.list_in_memory_guardrails().len()
}

pub(super) fn guardrail_from_row(row: &GuardrailRow) -> Guardrail {
    Guardrail {
        guardrail_id: Some(row.guardrail_id.clone()),
        guardrail_name: row.guardrail_name.clone(),
        litellm_params: decode_litellm_params(
            row.litellm_params.as_deref().unwrap_or("{}").as_bytes(),
        )
        .expect("the stored params decode"),
        guardrail_info: None,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: row.team_id.clone(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
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

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn logged(captured: &Captured) -> String {
    let bytes = captured.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}
