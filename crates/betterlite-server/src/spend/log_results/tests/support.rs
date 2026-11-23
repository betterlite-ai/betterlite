use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::table_ops::rows::{RowDecoder as _, RowError, SpendLogRow};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use rust_decimal::Decimal;
use serde::Serialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::spend::log_payload::daily_summary::{DailySummaryRow, SummaryQuery};
use crate::spend::log_results::error::StoreError;
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::log_results::rows::SpendLogRowsQuery;
use crate::spend::log_results::session::{
    SessionAggregate, SessionAggregateError, SessionAggregateReader, SessionLookup,
};
use crate::spend::log_results::store::LogResultsStore;
use crate::spend::log_results::{LogResultsState, routes};

pub(super) type Events = Arc<Mutex<Vec<String>>>;

pub(super) fn record(events: &Events, event: String) {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event);
}

pub(super) fn seen(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

// a value through its wire JSON, the way a response body reaches a client
pub(super) fn as_json<T>(value: &T) -> JsonData
where
    T: Serialize,
{
    json(&serde_json::to_string(value).unwrap())
}

// which failure a fake store or reader answers with
#[derive(Debug, Clone, Copy)]
pub(super) enum Failure {
    Database,
    Row,
    Proxy,
    Http,
}

impl Failure {
    fn store_error(self) -> StoreError {
        match self {
            Self::Database => StoreError::Database(DbError::NoDbConnected),
            Self::Row => StoreError::Row(RowError::NotAnObject),
            Self::Proxy => StoreError::Proxy(Box::new(ProxyException::new(ProxyExceptionInit {
                message: "budget exceeded".to_owned(),
                error_type: "budget_exceeded".to_owned(),
                param: None,
                code: ExceptionCode::Status(429),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            }))),
            Self::Http => StoreError::Http {
                status: 403,
                detail: "no team budget".to_owned(),
            },
        }
    }

    pub(super) fn reader_error(self) -> SessionAggregateError {
        match self {
            Self::Row => SessionAggregateError::Row(serde_json::from_str::<u8>("no").unwrap_err()),
            Self::Database | Self::Proxy | Self::Http => {
                SessionAggregateError::Database(DbError::NoDbConnected)
            }
        }
    }
}

// the two listings the legacy handler reads plus the session aggregate port, recording each call
#[derive(Default)]
pub(super) struct Fake {
    pub(super) events: Events,
    pub(super) rows: Vec<SpendLogRow>,
    pub(super) summary: Vec<DailySummaryRow>,
    pub(super) aggregates: Vec<SessionAggregate>,
    pub(super) row_failure: Option<Failure>,
    pub(super) summary_failure: Option<Failure>,
    pub(super) aggregate_failure: Option<Failure>,
}

fn outcome<Item>(failure: Option<Failure>, values: &[Item]) -> Result<Vec<Item>, StoreError>
where
    Item: Clone,
{
    failure.map_or_else(|| Ok(values.to_vec()), |error| Err(error.store_error()))
}

#[async_trait]
impl LogResultsStore for Fake {
    async fn spend_logs(&self, query: &SpendLogRowsQuery) -> Result<Vec<SpendLogRow>, StoreError> {
        record(
            &self.events,
            format!("rows {} {}", query.params.join("|"), query.sql),
        );
        outcome(self.row_failure, &self.rows)
    }

    async fn daily_summary(
        &self,
        query: &SummaryQuery,
    ) -> Result<Vec<DailySummaryRow>, StoreError> {
        record(
            &self.events,
            format!("summary {} {}", query.params.join("|"), query.sql),
        );
        outcome(self.summary_failure, &self.summary)
    }
}

#[async_trait]
impl SessionAggregateReader for Fake {
    async fn session_aggregates(
        &self,
        lookup: &SessionLookup,
    ) -> Result<Vec<SessionAggregate>, SessionAggregateError> {
        record(
            &self.events,
            format!(
                "aggregate sessions={} keys={}",
                lookup.session_ids.join("|"),
                lookup.api_keys.join("|")
            ),
        );
        self.aggregate_failure.map_or_else(
            || Ok(self.aggregates.clone()),
            |error| Err(error.reader_error()),
        )
    }
}

pub(super) fn state(store: Option<Arc<dyn LogResultsStore>>) -> LogResultsState {
    LogResultsState { store }
}

pub(super) async fn call(state: LogResultsState, uri: &str, caller: Option<&Principal>) -> Reply {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    if let Some(principal) = caller {
        request.extensions_mut().insert(AuthContext {
            principal: principal.clone(),
        });
    }
    let response = routes(state).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply {
        status,
        headers,
        body,
    }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(&self) -> JsonData {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(super) fn header(&self, name: &str) -> Option<String> {
        let value = self.headers.get(name)?;
        Some(value.to_str().unwrap().to_owned())
    }
}

pub(super) fn row(request_id: &str, start: &str) -> SpendLogRow {
    let text = format!(
        "{{\"request_id\":\"{request_id}\",\"call_type\":\"acompletion\",\"api_key\":\"hashed\",\"startTime\":\"{start}\",\"endTime\":\"{start}\",\"created_at\":\"{start}\",\"updated_at\":\"{start}\"}}"
    );
    SpendLogRow::decode_row(&text).unwrap()
}

pub(super) fn list_row(
    request_id: &str,
    session_id: Option<&str>,
    api_key: Option<&str>,
) -> SpendLogListRow {
    let session = session_id.map_or_else(|| String::from("null"), |id| format!("\"{id}\""));
    let key = api_key.map_or_else(|| String::from("null"), |key| format!("\"{key}\""));
    let text = format!(
        "{{\"request_id\":\"{request_id}\",\"call_type\":\"acompletion\",\"api_key\":{key},\"startTime\":\"2026-01-01T00:00:00+00:00\",\"endTime\":\"2026-01-01T00:00:01+00:00\",\"session_id\":{session}}}"
    );
    serde_json::from_str(&text).unwrap()
}

pub(super) fn aggregate(session_id: &str, api_key: &str) -> SessionAggregate {
    SessionAggregate {
        session_id: session_id.to_owned(),
        api_key: api_key.to_owned(),
        session_total_count: 3,
        session_total_spend: decimal("1.5"),
        session_total_duration_ms: 1_200,
        mcp_tool_call_count: 0,
        mcp_tool_call_spend: Decimal::ZERO,
        session_cache_hit_count: 1,
        session_llm_count: 2,
        session_agent_count: 1,
        session_total_prompt_tokens: 10,
        session_completion_tokens: 20,
        session_total_tokens: 30,
        session_models: vec![String::from("m")],
        session_models_truncated: false,
    }
}

pub(super) fn summary_row(
    day: &str,
    api_key: Option<&str>,
    user: Option<&str>,
    model: &str,
    spend: &str,
) -> DailySummaryRow {
    DailySummaryRow {
        day: day.to_owned(),
        api_key: api_key.map(str::to_owned),
        user: user.map(str::to_owned),
        model: model.to_owned(),
        spend: SpendAmount(decimal(spend)),
    }
}

fn principal(roles: Vec<Role>, user_id: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: "virtual-key".to_owned(),
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
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey(None),
    }
}

pub(super) fn admin() -> Principal {
    principal(vec![Role::PlatformAdmin], Some("root"))
}

pub(super) fn viewer() -> Principal {
    principal(vec![Role::PlatformViewer], Some("root"))
}

pub(super) fn internal(user_id: &str) -> Principal {
    principal(Vec::new(), Some(user_id))
}

pub(super) fn internal_without_user() -> Principal {
    principal(Vec::new(), None)
}
