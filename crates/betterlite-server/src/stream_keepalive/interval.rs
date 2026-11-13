use betterlite_config::sections::settings::JsonValue;

use super::deployment::deployment_keepalive;
use super::seconds::{parse_float_text, setting_seconds, value_seconds};
use super::{JsonObject, KEEPALIVE_MAX_SECONDS, KEEPALIVE_MIN_SECONDS, KeepaliveRouter};
use crate::keepalive_interval::PingSetting;

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
        tracing::info!("keepalive_seconds={value} clamped to {clamped} [min=1.0, max=300.0]");
    }
    clamped
}
