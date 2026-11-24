use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use rust_decimal::Decimal;
use serde::Serialize;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::log_results::session::{
    SessionAggregate, SessionAggregateError, SessionAggregateReader, SessionLookup,
};
use crate::spend::sessions::expansion::SessionTeamScope;
use crate::spend::sessions::params::SessionQuery;
use crate::spend::sessions::ports::{SessionGroupRow, SessionsStore, SessionsStoreError};
use crate::spend::sessions::rows::SessionSpendRow;
use crate::spend::sessions::{SessionsState, routes};
use crate::team_access::caller::TeamCaller;

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

// a value through its wire JSON, the way a response body reaches a client
pub(super) fn as_json<T>(value: &T) -> JsonData
where
    T: Serialize,
{
    json(&serde_json::to_string(value).unwrap())
}

// which failure the fake store answers with
#[derive(Debug, Clone, Copy)]
pub(super) enum Failure {
    Database,
    Row,
}

impl Failure {
    fn error(self) -> SessionsStoreError {
        match self {
            Self::Database => SessionsStoreError::Database(DbError::NoDbConnected),
            Self::Row => SessionsStoreError::Row(RowError::NotAnObject),
        }
    }
}

// every session read the two pagers make, recording the kind, the bound values and the SQL
#[derive(Default)]
pub(super) struct Fake {
    pub(super) events: Events,
    pub(super) groups: Vec<SessionGroupRow>,
    pub(super) group_count: i64,
    pub(super) representatives: Vec<SpendLogListRow>,
    pub(super) rows: Vec<SessionSpendRow>,
    pub(super) row_count: i64,
    pub(super) failure: Option<Failure>,
}

fn outcome<Item>(failure: Option<Failure>, values: &[Item]) -> Result<Vec<Item>, SessionsStoreError>
where
    Item: Clone,
{
    failure.map_or_else(|| Ok(values.to_vec()), |error| Err(error.error()))
}

impl Fake {
    fn recorded<Item>(
        &self,
        kind: &str,
        query: &SessionQuery,
        values: &[Item],
    ) -> Result<Vec<Item>, SessionsStoreError>
    where
        Item: Clone,
    {
        record(
            &self.events,
            format!("{kind} params={:?} {}", query.params, query.sql),
        );
        outcome(self.failure, values)
    }

    fn counted(
        &self,
        kind: &str,
        query: &SessionQuery,
        count: i64,
    ) -> Result<i64, SessionsStoreError> {
        let event = format!("{kind} params={:?} {}", query.params, query.sql);
        record(&self.events, event);
        self.failure.map_or(Ok(count), |error| Err(error.error()))
    }
}

#[async_trait]
impl SessionsStore for Fake {
    async fn session_groups(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionGroupRow>, SessionsStoreError> {
        self.recorded("groups", query, &self.groups)
    }

    async fn session_group_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        self.counted("group_count", query, self.group_count)
    }

    async fn session_representatives(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, SessionsStoreError> {
        self.recorded("representatives", query, &self.representatives)
    }

    async fn session_rows(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionSpendRow>, SessionsStoreError> {
        self.recorded("rows", query, &self.rows)
    }

    async fn session_row_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        self.counted("row_count", query, self.row_count)
    }
}

// the permitted teams B1704 would resolve, or the failure that narrows the caller to own rows
#[derive(Default)]
pub(super) struct Scope {
    pub(super) events: Events,
    pub(super) teams: Vec<String>,
    pub(super) failure: bool,
}

#[async_trait]
impl SessionTeamScope for Scope {
    async fn team_scope(&self, caller: &TeamCaller) -> Vec<String> {
        record(
            &self.events,
            format!("scope {}", caller.user_id.as_deref().unwrap_or("null")),
        );
        if self.failure {
            return Vec::new();
        }
        self.teams.clone()
    }
}

// the kind of each session read, in the order it ran
pub(super) fn kinds(events: &Events) -> Vec<String> {
    seen(events)
        .iter()
        .map(|event| event.split_once(' ').unwrap().0.to_owned())
        .collect()
}

// the session aggregate B1730 adds to each representative, recording the pairs it was asked for
#[derive(Default)]
pub(super) struct Aggregates {
    pub(super) events: Events,
    pub(super) aggregates: Vec<SessionAggregate>,
}

#[async_trait]
impl SessionAggregateReader for Aggregates {
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
        Ok(self.aggregates.clone())
    }
}

pub(super) fn state(
    store: Option<Arc<dyn SessionsStore>>,
    scope: Option<Arc<dyn SessionTeamScope>>,
) -> SessionsState {
    SessionsState { store, scope }
}

pub(super) async fn call(state: SessionsState, uri: &str, caller: Option<&Principal>) -> Reply {
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
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Reply { status, body }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(&self) -> JsonData {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub(super) fn group(session_key: &str, api_key: &str, last_activity: &str) -> SessionGroupRow {
    SessionGroupRow {
        session_key: session_key.to_owned(),
        api_key: api_key.to_owned(),
        last_activity: last_activity.to_owned(),
    }
}

pub(super) fn representative(
    request_id: &str,
    session_id: &str,
    api_key: &str,
    metadata: &str,
) -> SpendLogListRow {
    let text = format!(
        "{{\"request_id\":\"{request_id}\",\"call_type\":\"acompletion\",\"api_key\":\"{api_key}\",\"startTime\":\"2026-01-01T00:00:00\",\"endTime\":\"2026-01-01T00:00:01\",\"session_id\":\"{session_id}\",\"metadata\":{metadata},\"request_tags\":\"[{{\\\"team\\\":\\\"a\\\"}}]\"}}"
    );
    serde_json::from_str(&text).unwrap()
}

pub(super) fn spend_row(
    request_id: &str,
    session_id: &str,
    start: &str,
    user: Option<&str>,
    team_id: Option<&str>,
) -> SessionSpendRow {
    let user_json = user.map_or_else(|| String::from("null"), |id| format!("\"{id}\""));
    let team_json = team_id.map_or_else(|| String::from("null"), |id| format!("\"{id}\""));
    let text = format!(
        "{{\"request_id\":\"{request_id}\",\"call_type\":\"acompletion\",\"api_key\":\"hashed\",\"startTime\":\"{start}\",\"endTime\":\"{start}\",\"session_id\":\"{session_id}\",\"user\":{user_json},\"team_id\":{team_json},\"metadata\":\"{{\\\"status\\\":\\\"success\\\"}}\",\"request_tags\":\"[]\"}}"
    );
    serde_json::from_str(&text).unwrap()
}

fn principal(roles: Vec<Role>, user_id: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: String::from("virtual-key"),
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

pub(super) fn internal(user_id: &str) -> Principal {
    principal(Vec::new(), Some(user_id))
}

pub(super) fn internal_without_user() -> Principal {
    principal(Vec::new(), None)
}

pub(super) fn org_admin() -> Principal {
    principal(vec![Role::OrgAdmin], Some("olivia"))
}

pub(super) fn aggregate(session_id: &str, api_key: &str) -> SessionAggregate {
    SessionAggregate {
        session_id: session_id.to_owned(),
        api_key: api_key.to_owned(),
        session_total_count: 1,
        session_total_spend: Decimal::ZERO,
        session_total_duration_ms: 1_000,
        mcp_tool_call_count: 0,
        mcp_tool_call_spend: Decimal::ZERO,
        session_cache_hit_count: 0,
        session_llm_count: 1,
        session_agent_count: 0,
        session_total_prompt_tokens: 1,
        session_completion_tokens: 1,
        session_total_tokens: 2,
        session_models: vec![String::from("m")],
        session_models_truncated: false,
    }
}
