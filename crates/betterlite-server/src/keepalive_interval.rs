use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;

#[derive(Debug, Clone, PartialEq)]
pub enum PingSetting {
    Seconds(f64),
    Text(String),
}

#[must_use]
pub fn coerce_keepalive_interval(setting: Option<&PingSetting>) -> Option<Duration> {
    let seconds = match setting? {
        PingSetting::Seconds(seconds) => *seconds,
        PingSetting::Text(text) => parse_float_text(text)?,
    };
    if !seconds.is_finite() || seconds <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(seconds).ok()
}

#[must_use]
pub fn keepalive_seconds_of(deployment: &JsonValue) -> Option<PingSetting> {
    let params = deployment.get("litellm_params")?;
    match params.get("keepalive_seconds")? {
        JsonValue::Number(number) => number.as_f64().map(PingSetting::Seconds),
        JsonValue::String(text) => Some(PingSetting::Text(text.clone())),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

#[must_use]
pub fn agreed_keepalive_interval(
    deployments: &[JsonValue],
    global_interval: Option<&PingSetting>,
) -> Option<Duration> {
    let mut distinct: Vec<Option<PingSetting>> = Vec::new();
    for deployment in deployments {
        let value = keepalive_seconds_of(deployment);
        if !distinct.contains(&value) {
            distinct.push(value);
        }
    }
    if let [Some(agreed)] = distinct.as_slice() {
        return coerce_keepalive_interval(Some(agreed));
    }
    coerce_keepalive_interval(global_interval)
}

#[must_use]
pub fn ttft_keepalive_interval(
    request_data: &JsonValue,
    deployments: &[JsonValue],
    global_interval: Option<&PingSetting>,
) -> Option<Duration> {
    if !matches!(request_data.get("stream"), Some(JsonValue::Bool(true))) {
        return None;
    }
    agreed_keepalive_interval(deployments, global_interval)
}

pub trait DeploymentLookup {
    fn deployments_for(&self, model_name: &str) -> Vec<JsonValue>;
}

#[must_use]
pub fn ttft_interval_for(
    lookup: Option<&dyn DeploymentLookup>,
    request_data: &JsonValue,
    global_interval: Option<&PingSetting>,
) -> Option<Duration> {
    if !matches!(request_data.get("stream"), Some(JsonValue::Bool(true))) {
        return None;
    }
    let deployments = match (lookup, request_data.get("model")) {
        (Some(resolver), Some(JsonValue::String(model))) => resolver.deployments_for(model),
        _ => Vec::new(),
    };
    agreed_keepalive_interval(&deployments, global_interval)
}

// Python's float() ignores surrounding whitespace and accepts single underscores between digits
fn parse_float_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let mut cleaned = String::with_capacity(trimmed.len());
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let after_digit = cleaned
                .chars()
                .next_back()
                .is_some_and(|previous| previous.is_ascii_digit());
            let before_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !after_digit || !before_digit {
                return None;
            }
            continue;
        }
        cleaned.push(character);
    }
    cleaned.parse::<f64>().ok()
}

mod tests;
