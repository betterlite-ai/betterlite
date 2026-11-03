mod tests;

use std::str::FromStr;

use super::error::RedisSettingsError;
use super::model::scalars::{BoolOrText, CoercionKind, PassthroughValue, Scalar};
use super::model::settings::Target;

pub fn coerce_number<T>(
    name: &str,
    value: Option<Scalar<T>>,
) -> Result<Option<T>, RedisSettingsError>
where
    T: FromStr,
{
    match value {
        None => Ok(None),
        Some(Scalar::Value(parsed)) => Ok(Some(parsed)),
        Some(Scalar::Text(text)) => Ok(betterlite_env::number::<T>(name, &text)?),
    }
}

pub fn coerce_bool(
    name: &str,
    value: Option<BoolOrText>,
) -> Result<Option<bool>, RedisSettingsError> {
    match value {
        None => Ok(None),
        Some(BoolOrText::Value(parsed)) => Ok(Some(parsed)),
        Some(BoolOrText::Text(text)) => Ok(betterlite_env::flag(name, &text)?),
    }
}

// Unknown passthrough values remain text; recognized fields use the shared parsers.
pub fn coerce_passthrough(
    name: &str,
    value: &mut PassthroughValue,
    target: Target,
) -> Result<bool, RedisSettingsError> {
    let (Some(kind), PassthroughValue::Text(text)) = (coercion_kind(name, target), &*value) else {
        return Ok(true);
    };
    let variable = format!("REDIS_{}", name.to_uppercase());
    match coerce_passthrough_value(kind, &variable, text)? {
        Some(coerced) => {
            *value = coerced;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn coerce_passthrough_value(
    kind: CoercionKind,
    name: &str,
    text: &str,
) -> Result<Option<PassthroughValue>, RedisSettingsError> {
    let parsed = match kind {
        CoercionKind::Int => betterlite_env::number::<i64>(name, text).map(|value| {
            value.map(|parsed| PassthroughValue::Number(serde_json::Number::from(parsed)))
        }),
        CoercionKind::Bool => {
            betterlite_env::flag(name, text).map(|value| value.map(PassthroughValue::Bool))
        }
    };
    Ok(parsed?)
}

fn coercion_kind(name: &str, target: Target) -> Option<CoercionKind> {
    match name {
        "retry_on_timeout"
        | "single_connection_client"
        | "decode_responses"
        | "legacy_responses"
        | "ssl_check_hostname"
        | "ssl_validate_ocsp"
        | "ssl_validate_ocsp_stapled" => Some(CoercionKind::Bool),
        "health_check_interval" | "socket_read_size" => Some(CoercionKind::Int),
        "cluster_error_retry_attempts" | "reinitialize_steps" if target == Target::Cluster => {
            Some(CoercionKind::Int)
        }
        "require_full_coverage" | "read_from_replicas" | "dynamic_startup_nodes"
            if target == Target::Cluster =>
        {
            Some(CoercionKind::Bool)
        }
        _ => None,
    }
}
