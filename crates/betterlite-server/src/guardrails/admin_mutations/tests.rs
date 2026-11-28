#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::future::{Future, ready};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

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
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, TableBinding as _, TableError,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_guardrails::guardrail_registry::build::GuardrailFactories;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
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
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tower::ServiceExt as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;

use super::body::{BodyError, WriteRequest, decode_patch_body, decode_write_request};
use super::error::MutationError;
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Failure {
    Create,
    Delete,
    Update,
    Find,
}

#[derive(Default)]
struct GuardrailStore {
    rows: BTreeMap<String, GuardrailRow>,
    created: usize,
    failures: BTreeSet<Failure>,
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
    master_caller()
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
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(context) = auth {
        builder = builder.extension(context);
    }
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

pub(super) fn field<'value>(value: &'value JsonData, name: &str) -> &'value JsonData {
    let JsonData::Object(fields) = value else {
        panic!("not an object: {value:?}");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the body has no {name}: {value:?}"))
}

pub(super) fn keys(value: &JsonData) -> Vec<String> {
    let JsonData::Object(fields) = value else {
        panic!("not an object: {value:?}");
    };
    fields.keys().cloned().collect()
}

pub(super) fn text(value: &JsonData) -> &str {
    let JsonData::String(text) = value else {
        panic!("not a string: {value:?}");
    };
    text
}

pub(super) fn detail(body: &str) -> String {
    text(field(&parse(body), "detail")).to_owned()
}

pub(super) fn first_issue(body: &str) -> JsonData {
    let parsed = parse(body);
    let detail = field(&parsed, "detail");
    let JsonData::Array(items) = detail else {
        panic!("the detail is not a list: {detail:?}");
    };
    items.first().cloned().expect("one issue")
}

#[test]
fn error_keeps_the_typed_cause() {
    let registry = MutationError::Registry {
        source: RegistryError::MissingGuardrailType,
    };
    let registry_cause = Error::source(&registry).expect("a registry cause");
    assert_eq!(registry_cause.to_string(), "guardrail_type is required");
    let rows = MutationError::ReviewRows {
        source: ReviewRowsError::MissingRow {
            guardrail_id: "g1".to_owned(),
        },
    };
    let rows_cause = Error::source(&rows).expect("a review rows cause");
    assert_eq!(
        rows_cause.to_string(),
        "Guardrail with ID g1 not found after the update"
    );
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

impl From<GuardrailRow> for ReviewRow {
    fn from(row: GuardrailRow) -> Self {
        Self {
            guardrail_id: row.guardrail_id,
            guardrail_name: row.guardrail_name,
            litellm_params: None,
            guardrail_info: None,
            team_id: row.team_id,
            status: row.status,
            submitted_at: Some(row.created_at),
            reviewed_at: None,
        }
    }
}

pub(super) fn review_row(id: &str, name: &str, params: Option<&str>, status: &str) -> ReviewRow {
    ReviewRow {
        litellm_params: params.map(parse),
        ..ReviewRow::from(stored_row(id, name, "{}", status))
    }
}

impl FakeGuardrails {
    pub(super) fn insert(&self, row: GuardrailRow) {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.rows.insert(row.guardrail_id.clone(), row);
    }

    pub(super) fn row(&self, id: &str) -> Option<GuardrailRow> {
        let store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.rows.get(id).cloned()
    }

    pub(super) fn rows(&self) -> Vec<GuardrailRow> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .values()
            .cloned()
            .collect()
    }

    pub(super) fn created(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .created
    }

    fn fail(&self, failure: Failure) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failures
            .insert(failure);
    }

    fn find_row(&self, id: &str) -> Result<Option<GuardrailRow>, RowsError> {
        if self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failures
            .contains(&Failure::Find)
        {
            return Err(RowsError::MissingRow);
        }
        Ok(self.row(id))
    }
}

impl GuardrailRows for FakeGuardrails {
    fn create(
        &self,
        row: NewGuardrailRow,
    ) -> impl Future<Output = Result<GuardrailRow, RowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.failures.contains(&Failure::Create) {
            return ready(Err(RowsError::MissingRow));
        }
        store.created = store.created.saturating_add(1);
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
        drop(store);
        ready(Ok(created))
    }

    fn delete(&self, guardrail_id: &str) -> impl Future<Output = Result<(), RowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.failures.contains(&Failure::Delete) || store.rows.remove(guardrail_id).is_none() {
            return ready(Err(RowsError::MissingRow));
        }
        drop(store);
        ready(Ok(()))
    }

    fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.failures.contains(&Failure::Update) {
            return ready(Err(RowsError::MissingRow));
        }
        let Some(existing) = store.rows.get_mut(guardrail_id) else {
            return ready(Ok(None));
        };
        existing.guardrail_name = row.guardrail_name;
        existing.litellm_params = Some(row.litellm_params);
        existing.guardrail_info = Some(row.guardrail_info);
        existing.updated_at = Timestamp::Aware(row.updated_at.fixed_offset());
        let updated = existing.clone();
        drop(store);
        ready(Ok(Some(updated)))
    }

    fn find_active(&self) -> impl Future<Output = Result<Vec<GuardrailRow>, RowsError>> + Send {
        ready(Ok(self
            .rows()
            .into_iter()
            .filter(|row| row.status == "active")
            .collect()))
    }

    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(self.find_row(guardrail_id))
    }

    fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        ready(Ok(self
            .rows()
            .into_iter()
            .find(|row| row.guardrail_name == guardrail_name)))
    }
}

impl FakeReviews {
    pub(super) fn insert(&self, row: ReviewRow) {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.rows.insert(row.guardrail_id.clone(), row);
    }

    pub(super) fn row(&self, id: &str) -> Option<ReviewRow> {
        let store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.rows.get(id).cloned()
    }

    fn lookup(&self, guardrail_id: &str) -> Result<Option<ReviewRow>, ReviewRowsError> {
        let store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.fail_find {
            return Err(ReviewRowsError::MissingRow {
                guardrail_id: guardrail_id.to_owned(),
            });
        }
        Ok(store.rows.get(guardrail_id).cloned())
    }

    pub(super) fn statuses(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .statuses
            .clone()
    }

    fn fail_set(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_set = true;
    }

    fn fail_find(&self) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_find = true;
    }
}

impl ReviewRows for FakeReviews {
    fn find_unique(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<ReviewRow>, ReviewRowsError>> + Send {
        ready(self.lookup(guardrail_id))
    }

    fn set_status(
        &self,
        guardrail_id: &str,
        status: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), ReviewRowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.fail_set {
            return ready(Err(ReviewRowsError::MissingRow {
                guardrail_id: guardrail_id.to_owned(),
            }));
        }
        store.statuses.push(status.to_owned());
        if let Some(row) = store.rows.get_mut(guardrail_id) {
            row.status = status.to_owned();
            row.reviewed_at = Some(Timestamp::Aware(now.fixed_offset()));
        }
        drop(store);
        ready(Ok(()))
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
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8(bytes).unwrap()
}
