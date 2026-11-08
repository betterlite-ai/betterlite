use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use futures_util::StreamExt as _;
use futures_util::stream::{Stream, unfold};
use tokio::time::{Instant, timeout};
use tracing::info;

use crate::keepalive_interval::PingSetting;

pub type JsonObject = BTreeMap<String, JsonValue>;

pub const KEEPALIVE_MIN_SECONDS: f64 = 1.0;
pub const KEEPALIVE_MAX_SECONDS: f64 = 300.0;
pub const KEEPALIVE_CACHE_TTL: Duration = Duration::from_secs(5);

pub trait KeepaliveRouter: Send + Sync {
    fn deployment_params(&self, model_id: &str) -> Option<JsonObject>;

    fn group_deployments(&self, model_name: Option<&str>) -> Vec<JsonValue>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentKeepalive {
    pub seconds: Option<JsonValue>,
    pub allow_client_override: bool,
}

#[must_use]
pub fn deployment_keepalive(
    router: Option<&dyn KeepaliveRouter>,
    request_data: &JsonObject,
    hidden: &JsonObject,
) -> Option<DeploymentKeepalive> {
    let model_key = hidden
        .get("model_id")
        .and_then(JsonValue::as_str)
        .filter(|model_id| !model_id.is_empty());
    if let Some(model_id) = model_key {
        let params = router?.deployment_params(model_id)?;
        return Some(DeploymentKeepalive {
            seconds: params
                .get("keepalive_seconds")
                .filter(|seconds| !seconds.is_null())
                .cloned(),
            allow_client_override: params.get("allow_client_keepalive_override")
                == Some(&JsonValue::Bool(true)),
        });
    }
    let model_name = request_data.get("model").and_then(JsonValue::as_str);
    let mut distinct: Vec<DeploymentKeepalive> = Vec::new();
    for deployment in router?.group_deployments(model_name) {
        let pair = group_pair(&deployment);
        let agrees = distinct.iter().any(|known| {
            known.allow_client_override == pair.allow_client_override
                && seconds_agree(known.seconds.as_ref(), pair.seconds.as_ref())
        });
        if !agrees {
            distinct.push(pair);
        }
    }
    match distinct.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn group_pair(deployment: &JsonValue) -> DeploymentKeepalive {
    let params = deployment
        .get("litellm_params")
        .and_then(JsonValue::as_object);
    let seconds = params
        .and_then(|fields| fields.get("keepalive_seconds"))
        .filter(|seconds| !seconds.is_null())
        .cloned();
    let allow_client_override = params
        .and_then(|fields| fields.get("allow_client_keepalive_override"))
        == Some(&JsonValue::Bool(true));
    DeploymentKeepalive {
        seconds,
        allow_client_override,
    }
}

fn seconds_agree(left: Option<&JsonValue>, right: Option<&JsonValue>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(JsonValue::Number(left_number)), Some(JsonValue::Number(right_number))) => {
            left_number.as_f64() == right_number.as_f64()
        }
        (Some(JsonValue::String(left_text)), Some(JsonValue::String(right_text))) => {
            left_text == right_text
        }
        (Some(left_value), Some(right_value)) => left_value == right_value,
        _ => false,
    }
}

#[must_use]
pub fn is_explicit_keepalive_disable(raw: Option<&JsonValue>) -> bool {
    match raw {
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|value| value <= 0.0_f64),
        Some(JsonValue::String(text)) => {
            parse_float_text(text).is_some_and(|value| value <= 0.0_f64)
        }
        _ => false,
    }
}

#[must_use]
pub fn resolve_keepalive_seconds(
    router: Option<&dyn KeepaliveRouter>,
    request_data: &JsonObject,
    hidden: &JsonObject,
    global: Option<&PingSetting>,
) -> f64 {
    let deployment = deployment_keepalive(router, request_data, hidden);
    let deployment_raw = deployment
        .as_ref()
        .and_then(|config| config.seconds.as_ref());
    if is_explicit_keepalive_disable(deployment_raw) {
        return 0.0_f64;
    }
    let client_raw = match deployment.as_ref() {
        Some(config) if config.allow_client_override => request_data
            .get("keepalive_seconds")
            .filter(|seconds| !seconds.is_null()),
        _ => None,
    };
    let value = client_raw
        .or(deployment_raw)
        .map_or_else(|| global.map_or(0.0_f64, setting_seconds), value_seconds);
    if value <= 0.0_f64 {
        return 0.0_f64;
    }
    // max(1.0, min(value, 300.0)) by hand: the float methods keep a NaN where LiteLLM gives 1.0
    let lowered = if KEEPALIVE_MAX_SECONDS < value {
        KEEPALIVE_MAX_SECONDS
    } else {
        value
    };
    let clamped = if lowered > KEEPALIVE_MIN_SECONDS {
        lowered
    } else {
        KEEPALIVE_MIN_SECONDS
    };
    if clamped.to_bits() != value.to_bits() {
        info!("keepalive_seconds={value} clamped to {clamped} [min=1.0, max=300.0]");
    }
    clamped
}

fn value_seconds(raw: &JsonValue) -> f64 {
    match raw {
        JsonValue::Number(number) => number.as_f64().unwrap_or(0.0_f64),
        JsonValue::String(text) => parse_float_text(text).unwrap_or(0.0_f64),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            0.0_f64
        }
    }
}

fn setting_seconds(setting: &PingSetting) -> f64 {
    match setting {
        PingSetting::Seconds(seconds) => *seconds,
        PingSetting::Text(text) => parse_float_text(text).unwrap_or(0.0_f64),
    }
}

fn parse_float_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let mut previous_is_digit = false;
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let next_is_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !previous_is_digit || !next_is_digit {
                return None;
            }
        }
        previous_is_digit = character.is_ascii_digit();
    }
    trimmed.replace('_', "").parse::<f64>().ok()
}

pub struct KeepaliveResolver {
    router: Option<Arc<dyn KeepaliveRouter>>,
    request_data: Arc<JsonObject>,
    global: Option<PingSetting>,
    last_model_id: Option<String>,
    last_value: f64,
    last_resolved_at: Instant,
}

impl KeepaliveResolver {
    #[must_use]
    pub fn new(
        router: Option<Arc<dyn KeepaliveRouter>>,
        request_data: Arc<JsonObject>,
        global: Option<PingSetting>,
    ) -> Self {
        Self {
            router,
            request_data,
            global,
            last_model_id: None,
            last_value: 0.0,
            last_resolved_at: Instant::now(),
        }
    }

    #[must_use]
    pub fn resolve(&mut self, hidden: &JsonObject) -> f64 {
        let model_key = hidden
            .get("model_id")
            .and_then(JsonValue::as_str)
            .filter(|model_id| !model_id.is_empty());
        let now = Instant::now();
        if let Some(model_id) = model_key {
            let within_ttl =
                now.saturating_duration_since(self.last_resolved_at) < KEEPALIVE_CACHE_TTL;
            if within_ttl && self.last_model_id.as_deref() == Some(model_id) {
                return self.last_value;
            }
        }
        let value = resolve_keepalive_seconds(
            self.router.as_deref(),
            &self.request_data,
            hidden,
            self.global.as_ref(),
        );
        if let Some(model_id) = model_key {
            self.last_model_id = Some(model_id.to_owned());
            self.last_value = value;
            self.last_resolved_at = now;
        }
        value
    }
}

pub trait KeepaliveSource {
    fn hidden_params(&self) -> Option<&JsonObject>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Beat<T> {
    Item(T),
    Ping,
}

struct KeepaliveState<S> {
    items: Pin<Box<S>>,
    resolver: KeepaliveResolver,
    wait: Option<Duration>,
}

// A resolved interval is 0 or 1..=300, so only an interval a caller builds by hand reaches the error
fn heartbeat_wait(seconds: f64) -> Option<Duration> {
    if seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

pub fn with_keepalive<S, T>(
    items: S,
    resolver: KeepaliveResolver,
    initial_seconds: f64,
) -> impl Stream<Item = Beat<T>>
where
    S: Stream<Item = T> + Send + 'static,
    T: KeepaliveSource,
{
    let start = KeepaliveState {
        items: Box::pin(items),
        resolver,
        wait: heartbeat_wait(initial_seconds),
    };
    unfold(start, |mut state| async move {
        match state.wait {
            None => {
                let item = state.items.next().await?;
                state.wait = heartbeat_wait(
                    state
                        .resolver
                        .resolve(item.hidden_params().unwrap_or(&JsonObject::new())),
                );
                Some((Beat::Item(item), state))
            }
            Some(wait) => match timeout(wait, state.items.next()).await {
                Ok(Some(item)) => {
                    state.wait = heartbeat_wait(
                        state
                            .resolver
                            .resolve(item.hidden_params().unwrap_or(&JsonObject::new())),
                    );
                    Some((Beat::Item(item), state))
                }
                Ok(None) => None,
                Err(_) => Some((Beat::Ping, state)),
            },
        }
    })
}

mod tests;
