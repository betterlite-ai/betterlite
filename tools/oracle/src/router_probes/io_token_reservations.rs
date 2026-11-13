mod fakes;

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_router::io_token_reservations::clock::{IoClock, minute_label};
use betterlite_router::io_token_reservations::counter::IoCounter;
use betterlite_router::io_token_reservations::error::{IoPreCallError, RateLimitExceeded};
use betterlite_router::io_token_reservations::limits::{
    IoDeployment, IoLimits, MinuteKeys, minute_keys,
};
use betterlite_router::io_token_reservations::reservation::{
    IoReservation, clear_reservation, read_reservation,
};
use betterlite_router::io_token_reservations::service::IoTokenReservations;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use self::fakes::{ProbeCounter, ProbeEstimator};
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
    pub request: RequestInput,
    pub estimate: i64,
    #[serde(default)]
    pub counters: CounterInput,
    #[serde(default)]
    pub litellm_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: Option<String>,
    pub model: Option<String>,
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestInput {
    #[serde(default)]
    pub messages: Option<Json>,
    #[serde(default)]
    pub prompt: Option<Json>,
    #[serde(default)]
    pub input: Option<Json>,
    #[serde(default)]
    pub max_tokens: Option<Json>,
    #[serde(default)]
    pub max_completion_tokens: Option<Json>,
    #[serde(default)]
    pub max_output_tokens: Option<Json>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterInput {
    #[serde(default)]
    pub itpm: i64,
    #[serde(default)]
    pub otpm: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(pub Box<OutputFields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputFields {
    pub reservation: Option<IoReservation>,
    pub counters: CounterValues,
    pub error: Option<ErrorFields>,
    pub metadata: Option<IndexMap<String, Json>>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CounterValues {
    pub itpm: i64,
    pub otpm: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorFields {
    pub status_code: u16,
    pub code: String,
    pub error_type: String,
    pub retry_after_seconds: u64,
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
        .block_on(scenario(input))
        .map_err(|message| invalid(&message))?;
    Ok(ProbeOutput::IoTokenReservations(output))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

async fn scenario(input: &Input) -> Result<Output, String> {
    let payload = &input.0;
    let now = UNIX_EPOCH
        .checked_add(Duration::from_millis(payload.now_millis))
        .unwrap_or(UNIX_EPOCH);
    let deployment = deployment(payload);
    let mut request = request_body(payload);
    let counter = Arc::new(ProbeCounter::default());
    let keys = minute_keys(&deployment, &minute_label(now));
    seed(&counter, keys.as_ref(), payload.counters);
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let reservations = reservations(
        Arc::clone(&counter),
        payload.estimate,
        catalog,
        Arc::new(move || now),
    );
    let error = match reservations.pre_call_check(&deployment, &mut request).await {
        Ok(_) => None,
        Err(IoPreCallError::RateLimit(_)) => Some(error_fields()),
        Err(other) => return Err(other.to_string()),
    };
    let counters = keys
        .as_ref()
        .map_or_else(CounterValues::default, |current_keys| CounterValues {
            itpm: counter.value(&current_keys.itpm),
            otpm: counter.value(&current_keys.otpm),
        });
    Ok(Output(Box::new(OutputFields {
        reservation: read_reservation(&request),
        counters,
        error,
        metadata: object(request.get("metadata")),
        litellm_metadata: object(request.get("litellm_metadata")),
    })))
}

fn request_body(payload: &Payload) -> IndexMap<String, Json> {
    let mut request = request_body_from(&payload.request);
    if let Some(litellm_metadata) = &payload.litellm_metadata {
        request.insert(
            "litellm_metadata".to_owned(),
            Json::Object(litellm_metadata.clone()),
        );
    }
    clear_reservation(&mut request);
    request
}

fn reservations<T>(
    counter: Arc<T>,
    estimate: i64,
    catalog: Arc<ModelCatalog>,
    clock: IoClock,
) -> IoTokenReservations
where
    T: IoCounter + 'static,
{
    IoTokenReservations {
        counter,
        estimator: Arc::new(ProbeEstimator { estimate }),
        catalog,
        clock,
    }
}

fn deployment(payload: &Payload) -> IoDeployment<'_> {
    IoDeployment {
        model_id: payload.deployment.id.as_deref(),
        model: payload.deployment.model.as_deref(),
        limits: IoLimits {
            itpm: payload.deployment.itpm,
            otpm: payload.deployment.otpm,
        },
    }
}

fn seed(counter: &ProbeCounter, keys: Option<&MinuteKeys>, counters: CounterInput) {
    if let Some(current_keys) = keys {
        counter.seed(&current_keys.itpm, counters.itpm);
        counter.seed(&current_keys.otpm, counters.otpm);
    }
}

fn request_body_from(input: &RequestInput) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    for (key, field) in [
        ("messages", &input.messages),
        ("prompt", &input.prompt),
        ("input", &input.input),
        ("max_tokens", &input.max_tokens),
        ("max_completion_tokens", &input.max_completion_tokens),
        ("max_output_tokens", &input.max_output_tokens),
    ] {
        if let Some(value) = field {
            body.insert((*key).to_owned(), value.clone());
        }
    }
    body
}

fn object(value: Option<&Json>) -> Option<IndexMap<String, Json>> {
    match value {
        Some(Json::Object(metadata)) => Some(metadata.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Null)
        | None => None,
    }
}

fn error_fields() -> ErrorFields {
    ErrorFields {
        status_code: RateLimitExceeded::STATUS_CODE,
        code: RateLimitExceeded::CODE.to_owned(),
        error_type: RateLimitExceeded::ERROR_TYPE.to_owned(),
        retry_after_seconds: RateLimitExceeded::RETRY_AFTER_SECONDS,
    }
}
