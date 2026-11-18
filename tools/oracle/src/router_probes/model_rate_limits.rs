mod fakes;

use std::sync::{Arc, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::io_token_reservations::clock::minute_label;
use betterlite_router::model_rate_limits::cache::RateLimitCache;
use betterlite_router::model_rate_limits::error::{ModelRateLimitError, RateLimitExceeded};
use betterlite_router::model_rate_limits::keys::cache_keys;
use betterlite_router::model_rate_limits::service::{ModelRateLimits, SuccessUsage};
use betterlite_tokenizer::reservation::RequestBody;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use tokio::runtime::Builder;

use self::fakes::FakeCache;
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs carries a box, so its largest variant does not grow with this payload
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub now_millis: u64,
    pub deployment: DeploymentInput,
    #[serde(default)]
    pub local: Counters,
    #[serde(default)]
    pub remote: Counters,
    #[serde(default)]
    pub redis: RedisMode,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: Option<String>,
    pub model: Option<String>,
    pub model_name: String,
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
    pub itpm: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counters {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedisMode {
    #[default]
    None,
    Up,
    Breaker,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Check,
    Success,
    Redis,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub kind: StepKind,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub redis: RedisMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(pub Box<StepsOutput>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepsOutput {
    pub steps: Vec<StepOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutput {
    pub kind: StepKind,
    pub allowed: Option<bool>,
    pub error: Option<ErrorFields>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorFields {
    pub status_code: u16,
    pub code: String,
    pub error_type: String,
    pub retry_after_seconds: u64,
    pub message: String,
    pub content: String,
    pub num_retries: Option<u32>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let output = runtime
        .block_on(scenario(&input.0))
        .map_err(|message| invalid(&message))?;
    Ok(ProbeOutput::ModelRateLimits(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

async fn scenario(payload: &Payload) -> Result<Output, String> {
    let now = UNIX_EPOCH
        .checked_add(Duration::from_millis(payload.now_millis))
        .unwrap_or(UNIX_EPOCH);
    let deployment = deployment_spec(payload);
    let keys = cache_keys(&deployment, &minute_label(now));
    let cache = Arc::new(FakeCache::new(payload, &keys));
    let concrete = Arc::clone(&cache);
    let shared: Arc<dyn RateLimitCache> = concrete;
    let check = ModelRateLimits::new(shared, None, Arc::new(move || now));
    let mut steps = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        steps.push(run_step(&check, &cache, &deployment, step).await?);
    }
    Ok(Output(Box::new(StepsOutput { steps })))
}

async fn run_step(
    check: &ModelRateLimits,
    cache: &FakeCache,
    deployment: &DeploymentSpec,
    step: &Step,
) -> Result<StepOutput, String> {
    match step.kind {
        StepKind::Check => {
            let mut request = RequestBody::new();
            checked_step(check.async_pre_call_check(deployment, &mut request).await)
        }
        StepKind::Success => {
            success_step(check, step).await;
            Ok(step_output(step.kind))
        }
        StepKind::Redis => {
            if step.redis != RedisMode::None {
                *cache.mode.lock().unwrap_or_else(PoisonError::into_inner) = step.redis;
            }
            Ok(step_output(step.kind))
        }
    }
}

fn checked_step(result: Result<(), ModelRateLimitError>) -> Result<StepOutput, String> {
    match result {
        Ok(()) => Ok(StepOutput {
            kind: StepKind::Check,
            allowed: Some(true),
            error: None,
        }),
        Err(ModelRateLimitError::Exceeded(exceeded)) => Ok(StepOutput {
            kind: StepKind::Check,
            allowed: Some(false),
            error: Some(error_fields(&exceeded)),
        }),
        Err(other) => Err(other.to_string()),
    }
}

async fn success_step(check: &ModelRateLimits, step: &Step) {
    let usage = SuccessUsage {
        model_id: step.model_id.as_deref(),
        model: step.model.as_deref(),
        total_tokens: step.total_tokens,
    };
    check.async_log_success_event(&usage).await;
}

const fn step_output(kind: StepKind) -> StepOutput {
    StepOutput {
        kind,
        allowed: None,
        error: None,
    }
}

fn deployment_spec(payload: &Payload) -> DeploymentSpec {
    let input = &payload.deployment;
    let mut params = JsonObject::new();
    if let Some(model) = &input.model {
        params.insert("model".to_owned(), Json::Text(model.clone()));
    }
    let mut info = JsonObject::new();
    if let Some(id) = &input.id {
        info.insert("id".to_owned(), Json::Text(id.clone()));
    }
    let mut extra = JsonObject::new();
    for (field, limit) in [("tpm", input.tpm), ("rpm", input.rpm), ("itpm", input.itpm)] {
        if let Some(value) = limit {
            extra.insert(field.to_owned(), Json::Number(Number::from(value)));
        }
    }
    DeploymentSpec {
        model_name: input.model_name.clone(),
        litellm_params: params,
        model_info: info,
        extra,
    }
}

fn error_fields(exceeded: &RateLimitExceeded) -> ErrorFields {
    ErrorFields {
        status_code: RateLimitExceeded::STATUS_CODE,
        code: RateLimitExceeded::CODE.to_owned(),
        error_type: RateLimitExceeded::ERROR_TYPE.to_owned(),
        retry_after_seconds: RateLimitExceeded::RETRY_AFTER_SECONDS,
        message: format!("litellm.RateLimitError: {exceeded}"),
        content: exceeded.content(),
        num_retries: exceeded.num_retries,
    }
}
