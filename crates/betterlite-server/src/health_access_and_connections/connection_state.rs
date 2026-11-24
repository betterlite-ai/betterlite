use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::health_probe_policy::Settings as ProbePolicy;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::health_access_and_connections::visibility::HealthRouter;
use crate::health_check_entry::{HealthCheckEntry, HealthCheckFailure, HealthCheckOutcome};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::params::HealthProbeParams;
use crate::health_probe_settings::timeout::run_with_timeout;
use crate::model_write_fields::access::{ModelCall, ModelWriteAccess};
use crate::model_write_fields::error::ModelWriteError;
use crate::model_write_fields::teams::TeamRows;
use crate::team_access::access::TeamAccess;
use crate::team_access::caller::TeamCaller;

// the prompt and input the connection test always sends, whatever DEFAULT_HEALTH_CHECK_PROMPT says
pub const SELF_PROMPT: &str = "test from litellm";
// the separator only the ordinary failure text carries; the missing-mode result has neither key nor separator
const MISSING_MODE_MARKER: &str = "\nstacktrace: ";
// run_with_timeout's answer when ahealth_check outlives HEALTH_CHECK_TIMEOUT_SECONDS
const TIMEOUT_ERROR: &str = "Timeout exceeded";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ConnectionProbeError {
    pub message: String,
}

// ahealth_check, injected for one connection test: the probe params and mode go in, the result dict comes back
#[async_trait]
pub trait ConnectionProbe: Send + Sync {
    async fn probe(
        &self,
        params: &JsonObject,
        mode: Option<&str>,
    ) -> Result<JsonObject, ConnectionProbeError>;
}

// the deployment-level check on the final probe params, as can_user_make_model_call runs it
pub struct ModelCallAuthorizer {
    pub teams: Arc<dyn TeamRows>,
    pub team_access: Arc<TeamAccess>,
}

impl ModelCallAuthorizer {
    pub async fn authorize(
        &self,
        caller: &TeamCaller,
        team_id: Option<&str>,
    ) -> Result<(), ModelWriteError> {
        ModelWriteAccess {
            teams: self.teams.as_ref(),
            team_access: self.team_access.as_ref(),
        }
        .can_make_model_call(caller, team_id, ModelCall::Write)
        .await
    }
}

// what the route reads: router, probe, catalog, policy, the client-credential opt-in and the model management check
#[derive(Clone)]
pub struct TestConnectionState {
    pub router: Option<Arc<dyn HealthRouter>>,
    pub probe: Arc<dyn ConnectionProbe>,
    pub models: Arc<ModelCatalog>,
    pub policy: ProbePolicy,
    pub allow_client_side_credentials: bool,
    pub authorizer: Option<Arc<ModelCallAuthorizer>>,
}

// the production probe: ahealth_check through the shared entry, bounded by the health check timeout
pub struct EntryProbe {
    pub entry: Arc<HealthCheckEntry>,
    pub timeout: Duration,
}

#[async_trait]
impl ConnectionProbe for EntryProbe {
    // glue over check_health, which its own tests cover; the params conversion and timed_probe are tested here
    #[cfg_attr(test, mutants::skip)]
    async fn probe(
        &self,
        params: &JsonObject,
        mode: Option<&str>,
    ) -> Result<JsonObject, ConnectionProbeError> {
        let encoded = serde_json::to_value(params).map_err(|source| encode_error(&source))?;
        let map = serde_json::from_value(encoded).map_err(|source| encode_error(&source))?;
        let probe_params =
            HealthProbeParams::from_map(map).map_err(|source| probe_error(&source))?;
        let entry = Arc::clone(&self.entry);
        let probe_mode = mode.map(str::to_owned);
        let prompt = SELF_PROMPT.to_owned();
        let input = vec![SELF_PROMPT.to_owned()];
        let check = async move {
            entry
                .check_health(probe_params, probe_mode, Some(prompt), Some(input))
                .await
        };
        Box::pin(timed_probe(check, self.timeout)).await
    }
}

// run_with_timeout around ahealth_check: a timeout is the error result the route reports as "error"
pub async fn timed_probe<F>(
    probe: F,
    duration: Duration,
) -> Result<JsonObject, ConnectionProbeError>
where
    F: Future<Output = HealthCheckOutcome>,
{
    match run_with_timeout(probe, duration).await {
        Ok(outcome) => outcome_object(outcome),
        Err(_timeout) => Ok(timeout_object()),
    }
}

fn outcome_object(outcome: HealthCheckOutcome) -> Result<JsonObject, ConnectionProbeError> {
    match outcome {
        HealthCheckOutcome::Healthy(response) => {
            let encoded =
                serde_json::to_value(&response).map_err(|source| encode_error(&source))?;
            serde_json::from_value(encoded).map_err(|source| encode_error(&source))
        }
        HealthCheckOutcome::Failed(failure) => failure_object(&failure),
    }
}

// ahealth_check's except block: the error text, and for every ordinary failure the raw request it logged
fn failure_object(failure: &HealthCheckFailure) -> Result<JsonObject, ConnectionProbeError> {
    let mut result = JsonObject::new();
    result.insert("error".to_owned(), Json::Text(failure.error.clone()));
    let missing_mode = failure.raw_request.is_none() && failure.error.contains(MISSING_MODE_MARKER);
    if !missing_mode {
        let raw = match &failure.raw_request {
            Some(record) => {
                let encoded =
                    serde_json::to_value(record).map_err(|source| encode_error(&source))?;
                Json::Object(
                    serde_json::from_value(encoded).map_err(|source| encode_error(&source))?,
                )
            }
            None => Json::Null,
        };
        result.insert("raw_request_typed_dict".to_owned(), raw);
    }
    Ok(result)
}

fn timeout_object() -> JsonObject {
    let mut result = JsonObject::new();
    result.insert("error".to_owned(), Json::Text(TIMEOUT_ERROR.to_owned()));
    result
}

fn encode_error(source: &serde_json::Error) -> ConnectionProbeError {
    ConnectionProbeError {
        message: source.to_string(),
    }
}

fn probe_error(source: &HealthProbeError) -> ConnectionProbeError {
    ConnectionProbeError {
        message: source.to_string(),
    }
}
