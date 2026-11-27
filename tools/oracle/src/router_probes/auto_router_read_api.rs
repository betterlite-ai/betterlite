use std::fmt::Display;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;
use betterlite_server::auto_router_read_api::VIEW_ACTION;
use betterlite_server::auto_router_read_api::error::ReadApiError;
use betterlite_server::auto_router_read_api::query::{BenchmarksQuery, SessionQuery};
use betterlite_server::auto_router_read_api::service::{benchmarks_for, session_for};
use betterlite_server::auto_router_read_api::store::AutoRouterReadStore;
use betterlite_server::benchmark_aggregates::decode::decode_session_agg_rows;
use betterlite_server::benchmark_aggregates::row::SessionAggRow;
use betterlite_server::benchmark_aggregates::window::BenchmarkWindow;
use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::routing::auto_router_read_api::ConfiguredRouter;
use betterlite_server::shadow_eval_lifecycle::access::require_admin;
use betterlite_server::shadow_eval_validation::access::AdminRole;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enums in mod.rs would otherwise carry every field of this payload inline in their largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    now_unix_millis: i64,
    #[serde(default)]
    deployments: Vec<Deployment>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Benchmarks(BenchmarksStep),
    Session(SessionStep),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BenchmarksStep {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    database: bool,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    rows: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionStep {
    #[serde(default)]
    api_key: String,
    session_id: String,
    #[serde(default)]
    sessions: Vec<SessionSeed>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionSeed {
    last_turn_at: String,
    row: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct QueryBounds {
    start: String,
    end: String,
    api_key: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionLookup {
    api_key: String,
    session_id: String,
    matched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StepOutcome {
    Benchmarks {
        response: Value,
        query: QueryBounds,
    },
    Session {
        response: Value,
        lookup: SessionLookup,
    },
    Failed {
        status: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    steps: Vec<StepOutcome>,
}

fn invalid<T: Display + ?Sized>(error: &T) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

// the store the reads open, fed the payload's rows and recording what each read asked for
#[derive(Default)]
struct ProbeStore {
    rows: Vec<SessionAggRow>,
    sessions: Vec<(String, AutoRouterSession)>,
    query_bounds: Mutex<Option<QueryBounds>>,
    lookup: Mutex<Option<SessionLookup>>,
}

impl ProbeStore {
    fn take_query_bounds(&self) -> Option<QueryBounds> {
        self.query_bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn take_lookup(&self) -> Option<SessionLookup> {
        self.lookup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[async_trait]
impl AutoRouterReadStore for ProbeStore {
    async fn agg_rows(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError> {
        *self
            .query_bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(QueryBounds {
            start: window.query_start.clone(),
            end: window.query_end.clone(),
            api_key: api_key.map(str::to_owned),
            user_id: user_id.map(str::to_owned),
        });
        Ok(self.rows.clone())
    }

    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError> {
        let newest = self
            .sessions
            .iter()
            .filter(|(_, row)| row.hashed_api_key == api_key && row.session_id == session_id)
            .max_by(|(left, _), (right, _)| left.cmp(right))
            .map(|(_, row)| row.clone());
        *self.lookup.lock().unwrap_or_else(PoisonError::into_inner) = Some(SessionLookup {
            api_key: api_key.to_owned(),
            session_id: session_id.to_owned(),
            matched: newest.is_some(),
        });
        Ok(newest)
    }
}

fn decode_sessions(
    seeds: &[SessionSeed],
) -> Result<Vec<(String, AutoRouterSession)>, OperationError> {
    let mut sessions = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let bytes = serde_json::to_vec(&seed.row).map_err(|error| invalid(&error))?;
        let row: AutoRouterSession =
            serde_json::from_slice(&bytes).map_err(|error| invalid(&error))?;
        sessions.push((seed.last_turn_at.clone(), row));
    }
    Ok(sessions)
}

fn deployments(payload: &Payload) -> Vec<ConfiguredRouter> {
    payload
        .deployments
        .iter()
        .map(|deployment| ConfiguredRouter {
            model_name: deployment.model_name.clone().unwrap_or_default(),
            model: deployment.model.clone(),
        })
        .collect()
}

fn identity(role: Option<&str>) -> Identity {
    Identity {
        user_id: Some("probe".to_owned()),
        user_role: role.map(str::to_owned),
    }
}

fn probe_store(step: &BenchmarksStep) -> Result<ProbeStore, OperationError> {
    let row_bytes = serde_json::to_vec(&step.rows).map_err(|error| invalid(&error))?;
    Ok(ProbeStore {
        rows: decode_session_agg_rows(&row_bytes).map_err(|error| invalid(&error))?,
        ..ProbeStore::default()
    })
}

// FastAPI checks the role first, then the database, then the dates and the rollup read
async fn benchmarks_step(
    payload: &Payload,
    step: &BenchmarksStep,
    today: NaiveDate,
) -> Result<StepOutcome, OperationError> {
    if let Err(error) = require_admin(
        &identity(step.role.as_deref()),
        VIEW_ACTION,
        AdminRole::Viewer,
    ) {
        return Ok(StepOutcome::Failed {
            status: error.status_code(),
        });
    }
    if !step.database {
        return Ok(StepOutcome::Failed { status: 500 });
    }
    let store = probe_store(step)?;
    let query = BenchmarksQuery {
        start_date: step.start_date.clone(),
        end_date: step.end_date.clone(),
        api_key: step.api_key.clone(),
        user_id: step.user_id.clone(),
    };
    match benchmarks_for(&store, &query, &deployments(payload), today).await {
        Ok(response) => Ok(StepOutcome::Benchmarks {
            response: serde_json::to_value(response).map_err(|error| invalid(&error))?,
            query: store
                .take_query_bounds()
                .ok_or_else(|| invalid("the benchmarks read never reached the store"))?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}

// the session read needs no role: the key in the input is the caller's partition
async fn session_step(step: &SessionStep) -> Result<StepOutcome, OperationError> {
    let store = ProbeStore {
        sessions: decode_sessions(&step.sessions)?,
        ..ProbeStore::default()
    };
    let query = SessionQuery {
        session_id: step.session_id.clone(),
    };
    match session_for(&store, Some(step.api_key.as_str()), &query).await {
        Ok(response) => Ok(StepOutcome::Session {
            response: serde_json::to_value(response).map_err(|error| invalid(&error))?,
            lookup: store
                .take_lookup()
                .ok_or_else(|| invalid("the session read never reached the store"))?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}

async fn scenario(payload: &Payload) -> Result<Fields, OperationError> {
    let today = DateTime::<Utc>::from_timestamp_millis(payload.now_unix_millis)
        .map(|instant| instant.date_naive())
        .ok_or_else(|| invalid("the probe clock is outside the supported calendar"))?;
    let mut outcomes = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        outcomes.push(match step {
            Step::Benchmarks(benchmarks) => benchmarks_step(payload, benchmarks, today).await?,
            Step::Session(session) => session_step(session).await?,
        });
    }
    Ok(Fields { steps: outcomes })
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error))?;
    let fields = runtime.block_on(scenario(&input.0))?;
    Ok(ProbeOutput::AutoRouterReadApi(Output(Box::new(fields))))
}
