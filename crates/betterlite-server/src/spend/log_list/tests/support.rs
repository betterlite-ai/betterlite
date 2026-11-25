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
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::spend::log_list::authz::{Authorization, ScopeClause};
use crate::spend::log_list::conditions::{BuiltConditions, ConditionInputs, build_conditions};
use crate::spend::log_list::scope::LogListScope;
use crate::spend::log_list::store::{ListStoreError, LogListStore};
use crate::spend::log_list::{LogListState, routes};
use crate::spend::log_query::decode_spend_log_query;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::log_results::session::{
    SessionAggregate, SessionAggregateError, SessionAggregateReader, SessionLookup,
};
use crate::spend::sessions::params::SessionQuery;
use crate::spend::sessions::ports::{SessionGroupRow, SessionsStore, SessionsStoreError};
use crate::spend::sessions::rows::SessionSpendRow;
use crate::spend::visibility::error::VisibilityError;
use crate::spend::visibility::owners::OwnerPair;
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

// the kind of each recorded event, in the order it ran
pub(super) fn kinds(events: &Events) -> Vec<String> {
    seen(events)
        .iter()
        .map(|event| {
            event
                .split_once(' ')
                .map_or_else(|| event.clone(), |(kind, _)| kind.to_owned())
        })
        .collect()
}

pub(super) fn query_event(events: &Events, kind: &str) -> String {
    seen(events)
        .into_iter()
        .find(|event| event.starts_with(kind))
        .unwrap_or_default()
}

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(super) fn row(request_id: &str, user: Option<&str>, team_id: Option<&str>) -> SpendLogListRow {
    let user_field = user.map_or_else(String::new, |id| format!(r#","user":"{id}""#));
    let team_field = team_id.map_or_else(String::new, |id| format!(r#","team_id":"{id}""#));
    let text = format!(
        r#"{{"request_id":"{request_id}","call_type":"acompletion","spend":0.5,"model":"gpt-4","startTime":"2025-01-02T00:00:00Z","endTime":"2025-01-02T00:00:01Z"{user_field}{team_field}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

pub(super) fn metadata_row(request_id: &str, metadata: &str) -> SpendLogListRow {
    let mut built = row(request_id, Some("alice"), None);
    built.metadata = Some(JsonData::String(metadata.to_owned()));
    built.request_tags = Some(JsonData::String(String::from("[\"tag\"]")));
    built
}

// the store answers the two list reads and records each SQL and its bound values
pub(super) struct FakeStore {
    pub(super) events: Events,
    pub(super) count: i64,
    pub(super) rows: Vec<SpendLogListRow>,
    pub(super) fails: bool,
}

impl FakeStore {
    // each read records its SQL and its values, and answers the fixture's failure instead when one is asked for
    fn record_read(&self, kind: &str, query: &SessionQuery) -> Result<(), ListStoreError> {
        record(
            &self.events,
            format!("{kind} {:?} {}", query.params, query.sql),
        );
        if self.fails {
            return Err(ListStoreError::Database(DbError::NoDbConnected));
        }
        Ok(())
    }
}

#[async_trait]
impl LogListStore for FakeStore {
    async fn count_matches(&self, query: &SessionQuery) -> Result<i64, ListStoreError> {
        self.record_read("count", query)?;
        Ok(self.count)
    }

    async fn list_rows(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, ListStoreError> {
        self.record_read("rows", query)?;
        Ok(self.rows.clone())
    }
}

// B1731's reads behind the delegated grouped page
pub(super) struct FakeSessions {
    pub(super) events: Events,
    pub(super) groups: Vec<SessionGroupRow>,
    pub(super) representatives: Vec<SpendLogListRow>,
}

#[async_trait]
impl SessionsStore for FakeSessions {
    async fn session_groups(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionGroupRow>, SessionsStoreError> {
        record(
            &self.events,
            format!("groups {:?} {}", query.params, query.sql),
        );
        Ok(self.groups.clone())
    }

    async fn session_group_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        record(&self.events, format!("group_count {}", query.sql));
        Ok(i64::try_from(self.groups.len()).unwrap_or(0))
    }

    async fn session_representatives(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, SessionsStoreError> {
        record(&self.events, format!("representatives {}", query.sql));
        Ok(self.representatives.clone())
    }

    async fn session_rows(
        &self,
        _query: &SessionQuery,
    ) -> Result<Vec<SessionSpendRow>, SessionsStoreError> {
        Ok(Vec::new())
    }

    async fn session_row_count(&self, _query: &SessionQuery) -> Result<i64, SessionsStoreError> {
        Ok(0)
    }
}

// the two answers B1704 gives a test: an allowed owner pair and an allowed id precheck
#[derive(Clone, Copy)]
pub(super) struct Answers {
    pub(super) owners: bool,
    pub(super) precheck: bool,
}

impl Default for Answers {
    fn default() -> Self {
        Self {
            owners: true,
            precheck: true,
        }
    }
}

// how the fixture wires the list store: answering, absent, or failing every read
#[derive(Clone, Copy, Default)]
pub(super) enum StoreMode {
    #[default]
    Ready,
    Missing,
    Failing,
}

// B1704's checks with the answers a test pins down
pub(super) struct FakeScope {
    pub(super) events: Events,
    pub(super) permitted: Vec<String>,
    pub(super) teams: Vec<String>,
    pub(super) answers: Answers,
}

#[async_trait]
impl LogListScope for FakeScope {
    async fn assert_can_view_lookup(
        &self,
        _caller: &TeamCaller,
        request_id: &str,
    ) -> Result<(), VisibilityError> {
        let allowed = self.answers.precheck;
        record(&self.events, format!("precheck {request_id}"));
        if allowed {
            Ok(())
        } else {
            Err(VisibilityError::Forbidden {
                request_id: request_id.to_owned(),
            })
        }
    }

    async fn can_view_team(
        &self,
        _caller: &TeamCaller,
        team_id: &str,
    ) -> Result<bool, VisibilityError> {
        let mut visible = false;
        for allowed in &self.teams {
            if allowed == team_id {
                visible = true;
            }
        }
        record(&self.events, format!("team {team_id}"));
        Ok(visible)
    }

    async fn visible_team_ids(&self, _caller: &TeamCaller) -> Vec<String> {
        record(&self.events, String::from("permitted"));
        self.permitted.clone()
    }

    async fn can_view_row_owner(
        &self,
        _caller: &TeamCaller,
        owner: &OwnerPair,
    ) -> Result<bool, VisibilityError> {
        let visible = self.answers.owners;
        record(
            &self.events,
            format!("owner {:?} {:?}", owner.user, owner.team_id),
        );
        Ok(visible)
    }
}

pub(super) struct FakeEnricher {
    pub(super) events: Events,
}

#[async_trait]
impl SessionAggregateReader for FakeEnricher {
    async fn session_aggregates(
        &self,
        lookup: &SessionLookup,
    ) -> Result<Vec<SessionAggregate>, SessionAggregateError> {
        record(&self.events, format!("aggregate {:?}", lookup.session_ids));
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) store: StoreMode,
    pub(super) count: i64,
    pub(super) rows: Vec<SpendLogListRow>,
    pub(super) groups: Vec<SessionGroupRow>,
    pub(super) representatives: Vec<SpendLogListRow>,
    pub(super) permitted: Vec<String>,
    pub(super) teams: Vec<String>,
    pub(super) answers: Answers,
}

impl Fixture {
    // a fixture with the store mode and count a test pins, and no rows, groups or teams yet
    pub(super) fn new(store: StoreMode, count: i64) -> Self {
        Self {
            store,
            count,
            rows: Vec::new(),
            groups: Vec::new(),
            representatives: Vec::new(),
            permitted: Vec::new(),
            teams: Vec::new(),
            answers: Answers::default(),
        }
    }

    pub(super) fn with_rows(rows: Vec<SpendLogListRow>) -> Self {
        let count = i64::try_from(rows.len()).unwrap();
        let mut fixture = Self::new(StoreMode::Ready, count);
        fixture.rows = rows;
        fixture
    }

    // the four handles every test wires, with the store and the scope answering what the fixture pins
    fn state(&self, events: &Events) -> LogListState {
        let store: Option<Arc<dyn LogListStore>> = match self.store {
            StoreMode::Missing => None,
            StoreMode::Ready | StoreMode::Failing => Some(Arc::new(FakeStore {
                events: Arc::clone(events),
                count: self.count,
                rows: self.rows.clone(),
                fails: matches!(self.store, StoreMode::Failing),
            })),
        };
        let scope: Arc<dyn LogListScope> = Arc::new(FakeScope {
            events: Arc::clone(events),
            permitted: self.permitted.clone(),
            teams: self.teams.clone(),
            answers: self.answers,
        });
        LogListState {
            store,
            sessions: Some(Arc::new(FakeSessions {
                events: Arc::clone(events),
                groups: self.groups.clone(),
                representatives: self.representatives.clone(),
            })),
            scope: Some(scope),
            enricher: Some(Arc::new(FakeEnricher {
                events: Arc::clone(events),
            })),
        }
    }

    pub(super) async fn call(&self, uri: &str, caller: Option<&Principal>) -> (Reply, Events) {
        let events: Events = Events::default();
        let state = self.state(&events);
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
        (Reply { status, body }, events)
    }
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(&self) -> JsonData {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }
}

// the conditions a raw query produces without an authorization narrowing, for the SQL tests
pub(super) fn built_for(raw_query: &str, route: SpendLogRoute) -> BuiltConditions {
    let query = decode_spend_log_query(Some(raw_query));
    let prepared = query.prepare(route).unwrap();
    let choices = query.choices().unwrap();
    build_conditions(&ConditionInputs {
        query: &query,
        prepared: &prepared,
        choices: &choices,
        authorization: &Authorization {
            scope: ScopeClause::None,
        },
    })
}

pub(super) fn has(built: &BuiltConditions, fragment: &str) -> bool {
    built
        .conditions
        .iter()
        .any(|condition| condition.contains(fragment))
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
