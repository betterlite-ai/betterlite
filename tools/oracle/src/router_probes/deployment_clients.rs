use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::deployment_clients::Settings;
use betterlite_error::rate_limit::{RateLimitCategory, RateLimitDimension};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_clients::clients::CachedClients;
use betterlite_router::deployment_clients::credentials::{
    ClientsideCredentials, dynamic_litellm_params, is_clientside_credential,
};
use betterlite_router::deployment_clients::guard::{
    DeploymentSlot, LimitSource, MaxParallelRequestsExceeded, MaxParallelRequestsLimit,
    calculate_parallel_limit,
};
use betterlite_router::deployment_identity::stable_id::generate_model_id;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub calculate: Vec<CalculateInput>,
    pub limits: Vec<LimitInput>,
    pub cached: Vec<CachedInput>,
    pub credentials: Vec<CredentialInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalculateInput {
    pub max_parallel_requests: Option<i64>,
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
    pub default_max_parallel_requests: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitInput {
    pub max_parallel_requests: i64,
    pub model_id: String,
    pub model_group: String,
    pub steps: Vec<LimitStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LimitStep {
    Acquire,
    Release,
    EnterExit {
        #[serde(default)]
        fail: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedInput {
    pub model_id: String,
    pub model_group: String,
    pub max_parallel_requests: Option<i64>,
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
    pub default_max_parallel_requests: Option<i64>,
    pub lookups: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialInput {
    pub model_group: Option<String>,
    pub config: JsonObject,
    pub request: RequestFields,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestFields {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub calculate: Vec<Option<i64>>,
    pub limits: Vec<Vec<LimitObservation>>,
    pub cached: Vec<Vec<CachedObservation>>,
    pub credentials: Vec<CredentialObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitObservation {
    pub kind: String,
    pub in_flight: i64,
    pub error: Option<LimitError>,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitError {
    pub status_code: u16,
    pub code: String,
    pub error_type: String,
    pub category: RateLimitCategory,
    pub rate_limit_type: RateLimitDimension,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedObservation {
    pub installed: bool,
    pub present: bool,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialObservation {
    pub clientside: bool,
    pub params: JsonObject,
    pub api_key_sha256: Option<String>,
    pub model_id: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let calculate = input
        .calculate
        .iter()
        .map(|case| {
            calculate_parallel_limit(
                case.max_parallel_requests,
                case.rpm,
                case.tpm,
                case.default_max_parallel_requests,
            )
        })
        .collect();
    let limits = input.limits.iter().map(limit_outcome).collect();
    let cached = input.cached.iter().map(cached_outcome).collect();
    let credentials = input.credentials.iter().map(credential_outcome).collect();
    Ok(ProbeOutput::DeploymentClients(Output {
        calculate,
        limits,
        cached,
        credentials,
    }))
}

fn limit_outcome(input: &LimitInput) -> Vec<LimitObservation> {
    let limit = Arc::new(MaxParallelRequestsLimit::new(
        input.max_parallel_requests,
        input.model_id.clone(),
        input.model_group.clone(),
    ));
    let mut held: Vec<DeploymentSlot> = Vec::new();
    let mut observations = Vec::new();
    for step in &input.steps {
        let (kind, error, failed) = match step {
            LimitStep::Acquire => ("acquire", acquire_once(&limit, &mut held), false),
            LimitStep::Release => {
                drop(held.pop());
                ("release", None, false)
            }
            LimitStep::EnterExit { fail } => match limit.acquire() {
                Ok(slot) => {
                    drop(slot);
                    ("enter_exit", None, *fail)
                }
                Err(exceeded) => ("enter_exit", Some(limit_error(exceeded)), false),
            },
        };
        observations.push(LimitObservation {
            kind: kind.to_owned(),
            in_flight: limit.in_flight.load(Ordering::SeqCst),
            error,
            failed,
        });
    }
    observations
}

// the first acquisition decides the step, so the probe acquires once and keeps that guard
fn acquire_once(
    limit: &Arc<MaxParallelRequestsLimit>,
    held: &mut Vec<DeploymentSlot>,
) -> Option<LimitError> {
    match limit.acquire() {
        Ok(slot) => {
            held.push(slot);
            None
        }
        Err(error) => Some(limit_error(error)),
    }
}

fn limit_error(error: MaxParallelRequestsExceeded) -> LimitError {
    LimitError {
        status_code: MaxParallelRequestsExceeded::STATUS_CODE,
        code: MaxParallelRequestsExceeded::CODE.to_owned(),
        error_type: MaxParallelRequestsExceeded::ERROR_TYPE.to_owned(),
        category: MaxParallelRequestsExceeded::CATEGORY,
        rate_limit_type: MaxParallelRequestsExceeded::DIMENSION,
        message: error.to_string(),
    }
}

fn cached_outcome(input: &CachedInput) -> Vec<CachedObservation> {
    let clients = CachedClients::new(
        Duration::from_secs(Settings::default().client_ttl_seconds),
        Arc::new(|| UNIX_EPOCH),
    );
    let source = LimitSource {
        max_parallel_requests: input.max_parallel_requests,
        rpm: input.rpm,
        tpm: input.tpm,
        default_max_parallel_requests: input.default_max_parallel_requests,
    };
    let mut previous: Option<Arc<MaxParallelRequestsLimit>> = None;
    let mut observations = Vec::new();
    for _ in 0..input.lookups {
        let lookup = clients.max_parallel_requests(&input.model_id, &input.model_group, source);
        let reused = previous
            .as_ref()
            .zip(lookup.limit.as_ref())
            .is_some_and(|(before, now)| Arc::ptr_eq(before, now));
        previous = lookup.limit;
        observations.push(CachedObservation {
            installed: lookup.initialized,
            present: previous.is_some(),
            reused,
        });
    }
    observations
}

fn credential_outcome(input: &CredentialInput) -> CredentialObservation {
    let mut request_kwargs = JsonObject::new();
    for (key, value) in [
        ("api_key", &input.request.api_key),
        ("api_base", &input.request.api_base),
    ] {
        if let Some(text) = value {
            request_kwargs.insert((*key).to_owned(), Json::Text(text.clone()));
        }
    }
    let clientside = is_clientside_credential(&request_kwargs);
    let request = ClientsideCredentials {
        api_key: input.request.api_key.clone().map(SecretString::from),
        api_base: input.request.api_base.clone(),
        base_url: None,
    };
    let mut params = dynamic_litellm_params(input.config.clone(), &request);
    let model_id = input
        .model_group
        .as_deref()
        .filter(|_group| clientside)
        .map(|group| generate_model_id(group, &params));
    let api_key_sha256 = match params.shift_remove("api_key") {
        Some(Json::Text(key)) => Some(hex::encode(Sha256::digest(key.as_bytes()))),
        _ => None,
    };
    CredentialObservation {
        clientside,
        params,
        api_key_sha256,
        model_id,
    }
}
