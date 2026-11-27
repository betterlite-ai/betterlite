#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::extract::{Extension, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::autorouter_facts::session::bounded_session_id;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use rust_decimal::Decimal;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::{ReadState, benchmarks_read, session_read};
use crate::auth::auth_identity::AuthContext;
use crate::auto_router_read_api::error::ReadApiError;
use crate::auto_router_read_api::service::SessionResponse;
use crate::auto_router_read_api::store::AutoRouterReadStore;
use crate::benchmark_aggregates::group::Benchmarks;
use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::BenchmarkWindow;
use crate::shadow_eval_lifecycle::error::DB_NOT_CONNECTED;

#[derive(Debug, Deserialize)]
struct Detail {
    detail: String,
}

#[derive(Debug, Deserialize)]
struct Validation {
    detail: Vec<Issue>,
}

#[derive(Debug, Deserialize)]
struct Issue {
    #[serde(rename = "type")]
    kind: String,
    loc: Vec<String>,
    msg: String,
}

#[derive(Default)]
struct RecordingStore {
    rows: Vec<SessionAggRow>,
    sessions: Vec<AutoRouterSession>,
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingStore {
    fn with_session(session: AutoRouterSession) -> Self {
        Self {
            sessions: vec![session],
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AutoRouterReadStore for RecordingStore {
    async fn agg_rows(
        &self,
        _window: &BenchmarkWindow,
        _api_key: Option<&str>,
        _user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        Ok(self.rows.clone())
    }

    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((api_key.to_owned(), session_id.to_owned()));
        Ok(self
            .sessions
            .iter()
            .find(|row| row.hashed_api_key == api_key && row.session_id == session_id)
            .cloned())
    }
}

fn read_state(store: Option<Arc<dyn AutoRouterReadStore>>) -> Arc<ReadState> {
    Arc::new(ReadState {
        store,
        deployments: Vec::new(),
    })
}

fn principal(subject: &str, source_key: SourceKey) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: subject.to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key,
    }
}

fn master_principal() -> Principal {
    principal("master_key", SourceKey(None))
}

fn key_principal(fields: &str) -> Principal {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap();
    principal("key-owner", SourceKey(Some(key)))
}

fn authenticated(principal: Principal) -> Option<Extension<AuthContext>> {
    Some(Extension(AuthContext { principal }))
}

fn money(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn compared_row() -> SessionAggRow {
    SessionAggRow {
        router_name: "complexity-auto".to_owned(),
        router_type: "complexity".to_owned(),
        sessions: 1,
        turns: 2,
        spend: money("2.0"),
        saved_spend: money("2.0"),
        savings_estimated_turns: 2,
        savings_estimated_actual_spend: money("2.0"),
        savings_estimated_saved_spend: money("2.0"),
        classifier_cost_recorded_turns: 2,
        classifier_cost: money("0.5"),
        ..SessionAggRow::default()
    }
}

fn session_row(api_key: &str) -> AutoRouterSession {
    let at: RowTimestamp = serde_json::from_str("\"2024-01-02T00:00:00\"").unwrap();
    AutoRouterSession {
        hashed_api_key: api_key.to_owned(),
        session_id: "s1".to_owned(),
        router_name: "complexity-auto".to_owned(),
        router_type: "complexity".to_owned(),
        first_turn_at: at.clone(),
        last_turn_at: at,
        last_model: "gpt-4o".to_owned(),
        turns: 2,
        spend: money("1.5"),
        saved_spend: money("0.5"),
        estimated_turns: 2,
        estimated_actual_spend: money("1.0"),
        estimated_saved_spend: money("0.5"),
        estimated_baseline_models: BTreeMap::new(),
        classifier_cost: money("0.25"),
        tier_turns: BTreeMap::new(),
        baseline_models: BTreeMap::from([("m1".to_owned(), 2)]),
    }
}

async fn benches(
    store: Option<Arc<dyn AutoRouterReadStore>>,
    caller: Option<Extension<AuthContext>>,
    raw: Option<&str>,
) -> Response {
    benchmarks_read(
        State(read_state(store)),
        caller,
        RawQuery(raw.map(str::to_owned)),
    )
    .await
}

async fn session(
    store: Option<Arc<dyn AutoRouterReadStore>>,
    caller: Option<Extension<AuthContext>>,
    raw: Option<&str>,
) -> Response {
    session_read(
        State(read_state(store)),
        caller,
        RawQuery(raw.map(str::to_owned)),
    )
    .await
}

async fn decode<T: for<'de> Deserialize<'de>>(response: Response) -> T {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn both_reads_require_authentication() {
    assert_eq!(
        benches(None, None, None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        session(None, None, Some("session_id=s1")).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_member_cannot_read_benchmarks() {
    let store: Arc<dyn AutoRouterReadStore> = Arc::new(RecordingStore::default());
    let response = benches(
        Some(store),
        authenticated(key_principal(r#"{"user_role":"internal_user"}"#)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Detail = decode(response).await;
    assert_eq!(
        body.detail,
        "only proxy admin roles can view auto-router benchmarks across the deployment"
    );
}

#[tokio::test]
async fn an_admin_viewer_reads_the_benchmarks() {
    let store = RecordingStore {
        rows: vec![compared_row()],
        ..RecordingStore::default()
    };
    let response = benches(
        Some(Arc::new(store)),
        authenticated(key_principal(r#"{"user_role":"proxy_admin_viewer"}"#)),
        Some("start_date=2024-01-01&end_date=2024-01-02"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let answer: Benchmarks = decode(response).await;
    assert_eq!(answer.start_date, "2024-01-01");
    assert_eq!(answer.end_date, "2024-01-02");
    assert_eq!(answer.groups[0].router_name, "complexity-auto");
    assert_eq!(answer.groups[0].saved_pct, Some(money("50")));
    assert_eq!(answer.totals.saved_spend, Some(money("2.0")));
}

#[tokio::test]
async fn a_proxy_without_a_database_answers_500() {
    let response = benches(
        None,
        authenticated(master_principal()),
        Some("start_date=2024-01-01"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Detail = decode(response).await;
    assert_eq!(body.detail, DB_NOT_CONNECTED);
    let response = session(
        None,
        authenticated(master_principal()),
        Some("session_id=s1"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Detail = decode(response).await;
    assert_eq!(body.detail, DB_NOT_CONNECTED);
}

#[tokio::test]
async fn an_unparsable_day_is_a_400_detail() {
    let store: Arc<dyn AutoRouterReadStore> = Arc::new(RecordingStore::default());
    let response = benches(
        Some(store),
        authenticated(master_principal()),
        Some("start_date=2024-13-01"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Detail = decode(response).await;
    assert_eq!(
        body.detail,
        "Invalid date format: 2024-13-01. Expected: 'YYYY-MM-DD'"
    );
}

#[tokio::test]
async fn an_empty_user_filter_is_the_validation_envelope() {
    let store: Arc<dyn AutoRouterReadStore> = Arc::new(RecordingStore::default());
    let response = benches(
        Some(store),
        authenticated(master_principal()),
        Some("user_id="),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Validation = decode(response).await;
    assert_eq!(body.detail.len(), 1);
    assert_eq!(body.detail[0].kind, "string_too_short");
    assert_eq!(body.detail[0].loc, ["query", "user_id"]);
    assert_eq!(
        body.detail[0].msg,
        "String should have at least 1 character"
    );
}

#[tokio::test]
async fn the_session_read_binds_the_callers_canonical_key() {
    let canonical = hex::encode(Sha256::digest(b"sk-test"));
    let store = RecordingStore::with_session(session_row(&canonical));
    let response = session(
        Some(Arc::new(store)),
        authenticated(key_principal(r#"{"api_key":"sk-test"}"#)),
        Some("session_id=s1"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let answer: SessionResponse = decode(response).await;
    assert_eq!(answer.session_id, "s1");
    assert_eq!(answer.baseline_model.as_deref(), Some("m1"));
    assert_eq!(answer.saved_spend, Some(money("0.5")));
}

#[tokio::test]
async fn the_master_key_reads_under_its_stable_alias() {
    let store = Arc::new(RecordingStore::with_session(session_row(
        "litellm_proxy_master_key",
    )));
    let handle: Arc<dyn AutoRouterReadStore> = store.clone();
    let response = session(
        Some(handle),
        authenticated(master_principal()),
        Some("session_id=s1"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        store.calls(),
        [("litellm_proxy_master_key".to_owned(), "s1".to_owned())]
    );
}

#[tokio::test]
async fn a_missing_session_id_is_the_validation_envelope() {
    let store: Arc<dyn AutoRouterReadStore> = Arc::new(RecordingStore::default());
    let response = session(Some(store), authenticated(master_principal()), None).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Validation = decode(response).await;
    assert_eq!(body.detail[0].kind, "missing");
    assert_eq!(body.detail[0].loc, ["query", "session_id"]);
    assert_eq!(body.detail[0].msg, "Field required");
}

#[tokio::test]
async fn an_unknown_session_is_a_404_with_the_quoted_id() {
    let store: Arc<dyn AutoRouterReadStore> = Arc::new(RecordingStore::default());
    let response = session(
        Some(store),
        authenticated(master_principal()),
        Some("session_id=it%27s"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: Detail = decode(response).await;
    assert_eq!(
        body.detail,
        "No auto-routed turns recorded for session \"it's\" under this key"
    );
}

#[tokio::test]
async fn an_oversized_session_id_reaches_the_store_bounded() {
    let oversized = "a".repeat(257);
    let bounded = bounded_session_id(&oversized);
    let store = Arc::new(RecordingStore::with_session(session_row(&bounded)));
    let handle: Arc<dyn AutoRouterReadStore> = store.clone();
    let raw = format!("session_id={oversized}");
    let response = session(Some(handle), authenticated(master_principal()), Some(&raw)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        store.calls(),
        [("litellm_proxy_master_key".to_owned(), bounded)]
    );
}
