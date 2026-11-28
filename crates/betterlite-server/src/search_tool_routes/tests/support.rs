#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::search::SearchRequest;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::mcp_stored_contracts::JsonValue as StoredJson;
use betterlite_db::search_tools::convert::tool::SearchTool;
use betterlite_db::search_tools::record::SearchToolRecord;
use betterlite_db::search_tools::registry::SearchToolRegistry;
use betterlite_db::search_tools::table_client::{
    CreatedAtOrder, SearchToolCreate, SearchToolTableClient, SearchToolUpdate,
};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_openai::search::records::SearchResponse;
use betterlite_pyhost::json_values::JsonValue;
use chrono::{DateTime, Utc};
use tower::ServiceExt as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::super::routes;
use super::super::state::{
    ConfigSearchTools, ConnectionFailure, ConnectionSearch, PortError, PortFuture, ProviderCatalog,
    SearchToolDatabase, SearchToolPorts, SearchToolState, TeamObjectLookup,
};
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::search_tool_refresh::{RouterSearchRefresh, SearchRefreshError};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct Refused(pub(super) &'static str);

pub(super) fn parse(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

pub(super) fn moment(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

pub(super) fn fixed_now() -> DateTime<Utc> {
    moment(1_780_000_000)
}

pub(super) fn record(search_tool_id: &str, name: &str, params: &str) -> SearchToolRecord {
    SearchToolRecord {
        search_tool_id: search_tool_id.to_owned(),
        search_tool_name: name.to_owned(),
        litellm_params: serde_json::from_str(params).unwrap(),
        search_tool_info: Some(serde_json::from_str(r#"{"description":"stored"}"#).unwrap()),
        created_at: moment(1_770_000_000),
        updated_at: moment(1_775_000_000),
    }
}

pub(super) fn config_tool(text: &str) -> SearchTool {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Default)]
pub(super) struct FakeTable {
    pub(super) rows: Arc<Mutex<Vec<SearchToolRecord>>>,
    pub(super) broken: Arc<AtomicBool>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

impl FakeTable {
    fn open(&self, call: String) -> Result<MutexGuard<'_, Vec<SearchToolRecord>>, TableError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        if self.broken.load(Ordering::SeqCst) {
            return Err(TableError::Publish {
                message: "the table is down".to_owned(),
            });
        }
        Ok(self.rows.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn insert(&self, data: &SearchToolCreate) -> Result<SearchToolRecord, TableError> {
        let created = SearchToolRecord {
            search_tool_id: "tool-new".to_owned(),
            search_tool_name: data.search_tool_name.clone(),
            litellm_params: stored(&data.litellm_params),
            search_tool_info: Some(stored(&data.search_tool_info)),
            created_at: data.created_at,
            updated_at: data.updated_at,
        };
        self.open("create".to_owned())?.push(created.clone());
        Ok(created)
    }

    fn lookup(&self, search_tool_id: &str) -> Result<Option<SearchToolRecord>, TableError> {
        let rows = self.open(format!("find_unique:{search_tool_id}"))?;
        Ok(rows
            .iter()
            .find(|row| row.search_tool_id == search_tool_id)
            .cloned())
    }

    fn replace(
        &self,
        search_tool_id: &str,
        data: &SearchToolUpdate,
    ) -> Result<SearchToolRecord, TableError> {
        let mut rows = self.open(format!("update:{search_tool_id}"))?;
        let row = rows
            .iter_mut()
            .find(|row| row.search_tool_id == search_tool_id)
            .ok_or(TableError::MissingRow)?;
        row.search_tool_name.clone_from(&data.search_tool_name);
        row.litellm_params = stored(&data.litellm_params);
        row.search_tool_info = Some(stored(&data.search_tool_info));
        row.updated_at = data.updated_at;
        let updated = row.clone();
        drop(rows);
        Ok(updated)
    }

    fn remove(&self, search_tool_id: &str) -> Result<SearchToolRecord, TableError> {
        let mut rows = self.open(format!("delete:{search_tool_id}"))?;
        let index = rows
            .iter()
            .position(|row| row.search_tool_id == search_tool_id)
            .ok_or(TableError::MissingRow)?;
        Ok(rows.remove(index))
    }
}

pub(super) fn stored(text: &str) -> StoredJson {
    serde_json::from_str(text).unwrap()
}

impl SearchToolTableClient for FakeTable {
    fn create(
        &self,
        data: SearchToolCreate,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send {
        ready(self.insert(&data))
    }

    fn find_unique(
        &self,
        search_tool_id: &str,
    ) -> impl Future<Output = Result<Option<SearchToolRecord>, TableError>> + Send {
        ready(self.lookup(search_tool_id))
    }

    fn find_many(
        &self,
        order: Option<CreatedAtOrder>,
    ) -> impl Future<Output = Result<Vec<SearchToolRecord>, TableError>> + Send {
        ready(
            self.open(format!("find_many:{order:?}"))
                .map(|rows| rows.clone()),
        )
    }

    fn update(
        &self,
        search_tool_id: &str,
        data: SearchToolUpdate,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send {
        ready(self.replace(search_tool_id, &data))
    }

    fn delete(
        &self,
        search_tool_id: &str,
    ) -> impl Future<Output = Result<SearchToolRecord, TableError>> + Send {
        ready(self.remove(search_tool_id))
    }
}

#[derive(Clone)]
pub(super) struct NoReconnect;

impl DbReconnect for NoReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        _reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        Box::pin(ready(Ok(false)))
    }
}

pub(super) struct FakeRefresh {
    pub(super) calls: AtomicUsize,
    pub(super) fails: bool,
}

impl RouterSearchRefresh for FakeRefresh {
    fn refresh(&self) -> impl Future<Output = Result<(), SearchRefreshError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = if self.fails {
            Err(SearchRefreshError {
                source: Box::new(Refused("the router reload failed")),
            })
        } else {
            Ok(())
        };
        ready(outcome)
    }
}

pub(super) struct FakeConfig(Option<Vec<SearchTool>>);

impl ConfigSearchTools for FakeConfig {
    fn search_tools(&self) -> PortFuture<'_, Vec<SearchTool>, PortError> {
        let answer = self.0.clone().ok_or_else(|| PortError {
            source: Box::new(Refused("the config file is unreadable")),
        });
        Box::pin(ready(answer))
    }
}

#[derive(Clone, Copy)]
pub(super) enum TeamAnswer {
    Row(&'static str),
    Missing,
    NoDatabase,
}

pub(super) struct FakeTeams {
    pub(super) answer: TeamAnswer,
    pub(super) lookups: Mutex<Vec<String>>,
}

impl TeamObjectLookup for FakeTeams {
    fn team_object<'call>(
        &'call self,
        team_id: &'call str,
        _caller: &'call UserAPIKeyAuth,
    ) -> PortFuture<'call, LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.lookups.lock().unwrap().push(team_id.to_owned());
        let answer = match self.answer {
            TeamAnswer::Row(text) => Ok(serde_json::from_str(text).unwrap()),
            TeamAnswer::Missing => Err(TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            }),
            TeamAnswer::NoDatabase => Err(TeamLookupError::NoDatabase),
        };
        Box::pin(ready(answer))
    }
}

pub(super) struct FakeSearch {
    pub(super) answer: Result<usize, (&'static str, &'static str)>,
    pub(super) requests: Mutex<Vec<SearchRequest>>,
}

impl ConnectionSearch for FakeSearch {
    fn search(&self, request: SearchRequest) -> PortFuture<'_, SearchResponse, ConnectionFailure> {
        self.requests.lock().unwrap().push(request);
        let answer = match self.answer {
            Ok(count) => {
                let result =
                    r#"{"title":"Result","url":"https://result.example","snippet":"text"}"#;
                let results = vec![result; count].join(",");
                Ok(serde_json::from_str(&format!(r#"{{"results":[{results}]}}"#)).unwrap())
            }
            Err((error_type, message)) => Err(ConnectionFailure {
                error_type,
                source: Box::new(Refused(message)),
            }),
        };
        Box::pin(ready(answer))
    }
}

pub(super) struct FakeCatalog;

impl ProviderCatalog for FakeCatalog {
    fn ui_friendly_name(&self, provider: &str) -> Result<Option<String>, PortError> {
        match provider {
            "tavily" => Ok(None),
            "brave" => Err(PortError {
                source: Box::new(Refused("the brave config failed to build")),
            }),
            other => Ok(Some(format!("UI {other}"))),
        }
    }
}

pub(super) struct Fixture {
    pub(super) rows: Vec<SearchToolRecord>,
    pub(super) config: Option<Vec<SearchTool>>,
    pub(super) team: TeamAnswer,
    pub(super) search: Result<usize, (&'static str, &'static str)>,
    pub(super) database: bool,
    pub(super) refresh_fails: bool,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        rows: Vec::new(),
        config: Some(Vec::new()),
        team: TeamAnswer::Missing,
        search: Ok(2),
        database: true,
        refresh_fails: false,
    }
}

pub(super) struct Harness {
    pub(super) router: Router,
    pub(super) table: FakeTable,
    pub(super) refresh: Arc<FakeRefresh>,
    pub(super) teams: Arc<FakeTeams>,
    pub(super) search: Arc<FakeSearch>,
}

pub(super) fn build(setup: Fixture) -> Harness {
    let table = FakeTable::default();
    table.rows.lock().unwrap().extend(setup.rows);
    let refresh = Arc::new(FakeRefresh {
        calls: AtomicUsize::new(0),
        fails: setup.refresh_fails,
    });
    let teams = Arc::new(FakeTeams {
        answer: setup.team,
        lookups: Mutex::new(Vec::new()),
    });
    let search = Arc::new(FakeSearch {
        answer: setup.search,
        requests: Mutex::new(Vec::new()),
    });
    let database = setup.database.then(|| SearchToolDatabase {
        table: table.clone(),
        reconnect: NoReconnect,
        registry: SearchToolRegistry { now: fixed_now },
    });
    let shared_teams = Arc::clone(&teams);
    let teams_port: Arc<dyn TeamObjectLookup> = shared_teams;
    let shared_search = Arc::clone(&search);
    let search_port: Arc<dyn ConnectionSearch> = shared_search;
    let router = routes(SearchToolState {
        database,
        refresh: Arc::clone(&refresh),
        ports: Arc::new(SearchToolPorts {
            config: Arc::new(FakeConfig(setup.config)),
            teams: teams_port,
            search: search_port,
            catalog: Arc::new(FakeCatalog),
        }),
    });
    Harness {
        router,
        table,
        refresh,
        teams,
        search,
    }
}

pub(super) fn caller(principal: &str) -> AuthContext {
    let decoded: Principal = serde_json::from_str(principal).unwrap();
    AuthContext { principal: decoded }
}

pub(super) fn master() -> AuthContext {
    master_caller()
}

pub(super) fn key_caller(fields: &str) -> AuthContext {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap();
    let mut context = caller(
        r#"{"principal_type":"service_account","subject":"key-owner","auth_method":"api_key"}"#,
    );
    context.principal.source_key = SourceKey(Some(key));
    context
}

pub(super) async fn send(
    harness: &Harness,
    method: &str,
    uri: &str,
    body: &str,
    who: Option<AuthContext>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(context) = who {
        builder = builder.extension(context);
    }
    let request = builder.body(Body::from(body.to_owned())).unwrap();
    let response = harness.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

pub(super) fn names(body: &str) -> Vec<String> {
    parse(body)["search_tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["search_tool_name"].as_str().unwrap().to_owned())
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
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

pub(super) fn log_text(captured: &Captured) -> String {
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}
