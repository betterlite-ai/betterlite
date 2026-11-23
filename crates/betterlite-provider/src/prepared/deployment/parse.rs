use std::time::Duration;

use betterlite_models::controls::call_params::{DropParams, normalize_drop_params};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::headers::affinity::validate_affinity_header;

use super::{DEFAULT_TIMEOUT_SECONDS, DeploymentError};

pub(super) fn text(
    value: Option<JsonValue>,
    field: &'static str,
) -> Result<Option<String>, DeploymentError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text)),
        Some(_other) => Err(DeploymentError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

pub(super) fn seconds(value: Option<JsonValue>) -> Result<Duration, DeploymentError> {
    let given = match value {
        None | Some(JsonValue::Null) => {
            return Duration::try_from_secs_f64(DEFAULT_TIMEOUT_SECONDS).map_err(|source| {
                DeploymentError::Timeout {
                    value: DEFAULT_TIMEOUT_SECONDS,
                    source,
                }
            });
        }
        Some(given) => given,
    };
    let JsonValue::Number(number) = &given else {
        return Err(DeploymentError::FieldType {
            field: "timeout",
            expected: "a number of seconds",
        });
    };
    let Some(seconds) = number.as_f64() else {
        return Err(DeploymentError::FieldType {
            field: "timeout",
            expected: "a number of seconds",
        });
    };
    Duration::try_from_secs_f64(seconds).map_err(|source| DeploymentError::Timeout {
        value: seconds,
        source,
    })
}

pub(super) fn retries(value: Option<JsonValue>) -> Result<Option<u32>, DeploymentError> {
    let Some(given) = value else {
        return Ok(None);
    };
    match given {
        JsonValue::Null => Ok(None),
        JsonValue::Number(number) => retry_count(&number),
        JsonValue::Text(text) => retry_text(text),
        JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            Err(DeploymentError::FieldType {
                field: "max_retries",
                expected: "an integer",
            })
        }
    }
}

fn retry_count(number: &serde_json::Number) -> Result<Option<u32>, DeploymentError> {
    number
        .as_u64()
        .and_then(|count| u32::try_from(count).ok())
        .map(Some)
        .ok_or_else(|| DeploymentError::MaxRetries {
            value: number.to_string(),
        })
}

fn retry_text(text: String) -> Result<Option<u32>, DeploymentError> {
    text.parse::<u32>()
        .map(Some)
        .map_err(|_invalid| DeploymentError::MaxRetries { value: text })
}

pub(super) fn drop_params(value: Option<JsonValue>) -> Option<DropParams> {
    let given = match value {
        None | Some(JsonValue::Null) => return None,
        Some(JsonValue::Bool(flag)) => DropParams::Flag(flag),
        Some(JsonValue::Text(text)) => DropParams::Text(text),
        Some(other) => {
            tracing::warn!(
                value = ?other,
                "drop_params is not a flag value, treating it as unset"
            );
            return None;
        }
    };
    // a flag value wins, and any other string stays as written for the later policy step
    Some(normalize_drop_params(Some(&given)).map_or(given, DropParams::Flag))
}

pub(super) fn affinity_header(value: Option<JsonValue>) -> Result<Option<String>, DeploymentError> {
    let header = text(value, "provider_affinity_header")?;
    if let Some(name) = &header {
        validate_affinity_header(name).map_err(|source| {
            DeploymentError::ProviderAffinityHeader {
                value: name.clone(),
                source,
            }
        })?;
    }
    Ok(header)
}
