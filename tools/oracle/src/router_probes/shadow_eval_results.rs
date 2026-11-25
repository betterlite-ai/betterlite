mod store;

use std::fmt::Display;

use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::shadow_eval_models::job::ShadowJobResponse;
use betterlite_server::shadow_eval_results::access::require_viewer;
use betterlite_server::shadow_eval_results::error::ShadowResultsRouteError;
use betterlite_server::shadow_eval_results::query::{
    MAX_SHADOW_QUERY_BYTES, ShadowQueryError, decode_shadow_query,
};
use betterlite_server::shadow_eval_results::service::{get_shadow_eval_job, list_shadow_eval_jobs};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use self::store::ProbeStore;
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const VIEW_ACTION: &str = "view shadow evals";

// the probe enums in mod.rs hold this payload, and the box keeps their largest variant small
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Value>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    steps: Vec<StepOutcome>,
}

// a dumped response keeps the wire shape LiteLLM's model_dump_json writes, field for field
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum StepOutcome {
    Listed { jobs: Vec<Value> },
    Detailed { job: Value },
    Failed { status: u16 },
    OverLimit { bytes: usize, limit: usize },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListStep {
    #[serde(default)]
    role: Role,
    #[serde(default)]
    store: bool,
    #[serde(default)]
    query: String,
    #[serde(default)]
    pad_to: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailStep {
    #[serde(default)]
    role: Role,
    #[serde(default)]
    store: bool,
    job_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    List(ListStep),
    Detail(DetailStep),
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    #[default]
    Absent,
    ProxyAdmin,
    ProxyAdminViewer,
    InternalUser,
    Customer,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)?;
    let output = runtime.block_on(scenario(input))?;
    Ok(ProbeOutput::ShadowEvalResults(output))
}

async fn scenario(input: &Input) -> Result<Output, OperationError> {
    let payload = &input.0;
    let millis = payload
        .get("now_millis")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let now = timestamp(millis)?;
    let store = ProbeStore::build(payload)?;
    let steps: Vec<Step> = payload_optional(payload, "steps")?.unwrap_or_default();
    let mut outcomes = Vec::with_capacity(steps.len());
    for step in &steps {
        let outcome = match step {
            Step::List(list) => list_outcome(&store, now, list).await?,
            Step::Detail(detail) => detail_outcome(&store, now, detail).await?,
        };
        outcomes.push(outcome);
    }
    Ok(Output { steps: outcomes })
}

// FastAPI validates the query before the endpoint runs and checks the role before the database
async fn list_outcome(
    store: &ProbeStore,
    now: DateTime<Utc>,
    step: &ListStep,
) -> Result<StepOutcome, OperationError> {
    let query = match decode_shadow_query(&query_bytes(step)?) {
        Ok(query) => query,
        Err(ShadowQueryError::TooLarge { bytes, limit }) => {
            return Ok(StepOutcome::OverLimit { bytes, limit });
        }
        Err(ShadowQueryError::Invalid { .. }) => return Ok(StepOutcome::Failed { status: 422 }),
    };
    let visible = match viewer(store, step.role, step.store) {
        Ok(visible) => visible,
        Err(outcome) => return Ok(outcome),
    };
    match list_shadow_eval_jobs(visible, &query, now).await {
        Ok(jobs) => Ok(StepOutcome::Listed {
            jobs: jobs
                .iter()
                .map(|job| serde_json::to_value(job).map_err(invalid))
                .collect::<Result<_, _>>()?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}

async fn detail_outcome(
    store: &ProbeStore,
    now: DateTime<Utc>,
    step: &DetailStep,
) -> Result<StepOutcome, OperationError> {
    let identity = identity_of(step.role);
    match require_viewer(&identity, VIEW_ACTION, step.store.then_some(store)) {
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
        Ok(visible) => job_outcome(get_shadow_eval_job(visible, &step.job_id, now).await),
    }
}

fn job_outcome(
    result: Result<ShadowJobResponse, ShadowResultsRouteError>,
) -> Result<StepOutcome, OperationError> {
    match result {
        Ok(job) => Ok(StepOutcome::Detailed {
            job: serde_json::to_value(&job).map_err(invalid)?,
        }),
        Err(error) => Ok(StepOutcome::Failed {
            status: error.status_code(),
        }),
    }
}

fn viewer(store: &ProbeStore, role: Role, connected: bool) -> Result<&ProbeStore, StepOutcome> {
    let identity = identity_of(role);
    require_viewer(&identity, VIEW_ACTION, connected.then_some(store)).map_err(|error| {
        StepOutcome::Failed {
            status: error.status_code(),
        }
    })
}

fn query_bytes(step: &ListStep) -> Result<Vec<u8>, OperationError> {
    let mut bytes = step.query.clone().into_bytes();
    let total = step.pad_to.unwrap_or(bytes.len());
    if total < bytes.len() || total > MAX_SHADOW_QUERY_BYTES.saturating_add(1) {
        return Err(invalid(
            "pad_to must start at the query length and stay within the query limit",
        ));
    }
    bytes.resize(total, b'x');
    Ok(bytes)
}

fn identity_of(role: Role) -> Identity {
    Identity {
        user_id: None,
        user_role: role_name(role).map(str::to_owned),
    }
}

const fn role_name(role: Role) -> Option<&'static str> {
    match role {
        Role::Absent => None,
        Role::ProxyAdmin => Some("proxy_admin"),
        Role::ProxyAdminViewer => Some("proxy_admin_viewer"),
        Role::InternalUser => Some("internal_user"),
        Role::Customer => Some("customer"),
    }
}

fn payload_optional<T>(payload: &Value, name: &'static str) -> Result<Option<T>, OperationError>
where
    T: DeserializeOwned,
{
    match payload.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(invalid),
    }
}

fn timestamp(millis: i64) -> Result<DateTime<Utc>, OperationError> {
    DateTime::from_timestamp_millis(millis)
        .ok_or_else(|| invalid("a millisecond timestamp is not a valid instant"))
}

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
