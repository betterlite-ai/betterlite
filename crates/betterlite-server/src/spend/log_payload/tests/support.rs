use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::spend::log_payload::cold_storage::ColdStorageHandler;
use crate::spend::log_payload::dates::DetailDates;
use crate::spend::log_payload::ports::{ActiveLoggers, ColdStorageReader, PayloadLogger};
use crate::spend::log_payload::{LogPayloadState, routes};
use crate::spend::visibility::error::VisibilityError;
use crate::spend::visibility::owners::OwnerPair;
use crate::spend::visibility::query::{PayloadBinds, PayloadQuery};
use crate::spend::visibility::service::SpendLogVisibility;
use crate::spend::visibility::store::{PayloadRow, VisibilityStore};

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

pub(super) fn fields(text: &str) -> Fields {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

// a stored row, with the client call id it can also be found by
#[derive(Clone)]
pub(super) struct StoredRow {
    pub(super) row: PayloadRow,
    pub(super) call_id: Option<String>,
}

pub(super) fn stored_row(request_id: &str, call_id: Option<&str>, user: Option<&str>) -> StoredRow {
    StoredRow {
        row: PayloadRow {
            request_id: request_id.to_owned(),
            messages: Some(json(r#"[{"role":"user","content":"hi"}]"#)),
            response: Some(json(r#"{"id":"resp"}"#)),
            proxy_server_request: Some(json(r#"{"model":"m"}"#)),
            metadata: None,
            user: user.map(str::to_owned),
            team_id: None,
        },
        call_id: call_id.map(str::to_owned),
    }
}

// the spend log table: owner pairs and scoped id lookups the way the B1704 SQL answers them
pub(super) struct FakeTable {
    pub(super) rows: Vec<StoredRow>,
    pub(super) events: Events,
}

impl FakeTable {
    fn matching(&self, lookup_id: &str) -> impl Iterator<Item = &StoredRow> {
        self.rows.iter().filter(move |stored| {
            stored.row.request_id == lookup_id || stored.call_id.as_deref() == Some(lookup_id)
        })
    }
}

#[async_trait]
impl VisibilityStore for FakeTable {
    async fn owners(&self, lookup_id: &str) -> Result<Vec<OwnerPair>, VisibilityError> {
        record(&self.events, format!("owners {lookup_id}"));
        let owners: BTreeSet<OwnerPair> = self
            .matching(lookup_id)
            .map(|stored| OwnerPair {
                user: stored.row.user.clone(),
                team_id: stored.row.team_id.clone(),
            })
            .collect();
        Ok(owners.into_iter().collect())
    }

    async fn payload_rows(
        &self,
        query: &PayloadQuery<'_>,
    ) -> Result<Vec<PayloadRow>, VisibilityError> {
        let (lookup_id, user) = match query.binds {
            PayloadBinds::Admin((lookup_id,)) => (lookup_id, None),
            PayloadBinds::Owner((lookup_id, user))
            | PayloadBinds::OwnerOrTeams((lookup_id, user, _)) => (lookup_id, Some(user)),
        };
        record(&self.events, format!("payload_rows {lookup_id}"));
        let mut found: Vec<PayloadRow> = self
            .matching(lookup_id)
            .filter(|stored| {
                user.is_none_or(|owner| owner.is_some() && stored.row.user.as_deref() == owner)
            })
            .map(|stored| stored.row.clone())
            .collect();
        found.sort_by_key(|row| row.request_id != lookup_id);
        found.truncate(1);
        Ok(found)
    }

    async fn team(&self, _team_id: &str) -> Result<Option<LiteLLMTeamTable>, VisibilityError> {
        Ok(None)
    }

    async fn teams(&self, _team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
pub(super) enum Answer {
    Missing,
    Found(String),
    Fails,
}

// a payload callback that records each id and window it is asked for
pub(super) struct FakePayloadLogger {
    pub(super) name: &'static str,
    pub(super) answer: Answer,
    pub(super) events: Events,
}

#[async_trait]
impl PayloadLogger for FakePayloadLogger {
    async fn request_response_payload(
        &self,
        request_id: &str,
        window: &DetailDates,
    ) -> Result<Option<BTreeMap<String, JsonData>>, LoggerError> {
        let start = window.start_time_utc.map(|time| time.to_rfc3339());
        let end = window.end_time_utc.map(|time| time.to_rfc3339());
        record(
            &self.events,
            format!("callback {} {request_id} {start:?} {end:?}", self.name),
        );
        match &self.answer {
            Answer::Missing => Ok(None),
            Answer::Found(text) => Ok(Some(object(text))),
            Answer::Fails => Err(LoggerError::Delivery {
                callback: self.name.to_owned(),
                message: "the bucket is unreachable".to_owned(),
            }),
        }
    }
}

// a cold-storage logger that records each object key it is asked for
pub(super) struct FakeColdReader {
    pub(super) name: &'static str,
    pub(super) answer: Answer,
    pub(super) events: Events,
}

#[async_trait]
impl ColdStorageReader for FakeColdReader {
    async fn proxy_server_request(&self, object_key: &str) -> Result<Option<Fields>, LoggerError> {
        record(&self.events, format!("cold {} {object_key}", self.name));
        match &self.answer {
            Answer::Missing => Ok(None),
            Answer::Found(text) => Ok(Some(fields(text))),
            Answer::Fails => Err(LoggerError::Delivery {
                callback: self.name.to_owned(),
                message: "the object is gone".to_owned(),
            }),
        }
    }
}

#[derive(Default)]
pub(super) struct FakeLoggers {
    pub(super) payload: Vec<Arc<FakePayloadLogger>>,
    pub(super) cold: Vec<Arc<FakeColdReader>>,
}

impl ActiveLoggers for FakeLoggers {
    fn payload_loggers(&self) -> Vec<Arc<dyn PayloadLogger>> {
        self.payload
            .iter()
            .map(|logger| {
                let shared: Arc<dyn PayloadLogger> = Arc::<FakePayloadLogger>::clone(logger);
                shared
            })
            .collect()
    }

    fn cold_storage_logger(&self, callback_name: &str) -> Option<Arc<dyn ColdStorageReader>> {
        let found = self
            .cold
            .iter()
            .find(|reader| reader.name == callback_name)?;
        let shared: Arc<dyn ColdStorageReader> = Arc::<FakeColdReader>::clone(found);
        Some(shared)
    }
}

pub(super) fn cold_handler(
    loggers: Arc<FakeLoggers>,
    selected: Option<&str>,
) -> ColdStorageHandler {
    ColdStorageHandler {
        injected: None,
        selected: selected.map(str::to_owned),
        loggers,
    }
}

// users are never cached and there is no user table, so a non-admin is scoped to their own rows
struct NoUsers;

#[async_trait]
impl UserCache for NoUsers {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new(
            "the payload route never computes a budget reset",
        ))
    }
}

pub(super) fn visibility(table: FakeTable) -> SpendLogVisibility {
    let lookup = UserLookup {
        cache: Arc::new(NoUsers),
        store: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    };
    SpendLogVisibility {
        store: Arc::new(table),
        users: Arc::new(lookup),
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

// the route over its own router, with a database only when a table is given
pub(super) struct Fixture {
    pub(super) table: Option<Vec<StoredRow>>,
    pub(super) loggers: FakeLoggers,
    pub(super) selected: Option<&'static str>,
    pub(super) events: Events,
}

impl Fixture {
    pub(super) fn new(table: Option<Vec<StoredRow>>) -> Self {
        Self {
            table,
            loggers: FakeLoggers::default(),
            selected: None,
            events: Events::default(),
        }
    }

    pub(super) fn payload_logger(&mut self, name: &'static str, answer: Answer) {
        let events = Arc::clone(&self.events);
        self.loggers.payload.push(Arc::new(FakePayloadLogger {
            name,
            answer,
            events,
        }));
    }

    pub(super) fn cold_reader(&mut self, name: &'static str, answer: Answer) {
        let events = Arc::clone(&self.events);
        self.loggers.cold.push(Arc::new(FakeColdReader {
            name,
            answer,
            events,
        }));
        self.selected = Some(name);
    }

    pub(super) async fn call(self, uri: &str, caller: Option<&Principal>) -> Reply {
        let events = Arc::clone(&self.events);
        let loggers = Arc::new(self.loggers);
        let state = LogPayloadState {
            visibility: self
                .table
                .map(|rows| Arc::new(visibility(FakeTable { rows, events }))),
            loggers: Arc::<FakeLoggers>::clone(&loggers),
            cold_storage: Arc::new(cold_handler(loggers, self.selected)),
        };
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
}

pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(&self) -> JsonData {
        serde_json::from_slice(&self.body).unwrap()
    }
}
