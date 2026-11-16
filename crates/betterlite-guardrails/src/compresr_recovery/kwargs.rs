use betterlite_context::agentic_loop_plan::JsonMap;
use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};

use super::error::RecoveryError;
use crate::guardrail_deployment::markers::PRE_CALL_EXECUTED_KEY;
use crate::primitives::json_value::JsonValue;

const LOGGING_OBJECT_KEY: &str = "litellm_logging_obj";
const OWN_KEY_PREFIX: &str = "_compresr";
const METADATA_KEYS: [&str; 2] = ["metadata", "litellm_metadata"];

// Other guardrails' pre-call markers go so they inspect the restored originals; the own one stays to avoid recompressing
pub(super) fn follow_up_kwargs(
    kwargs: &JsonMap,
    own_marker: Option<&str>,
) -> Result<JsonMap, RecoveryError> {
    let mut sanitized: JsonMap = kwargs
        .iter()
        .filter(|(key, _)| !key.starts_with(OWN_KEY_PREFIX) && key.as_str() != LOGGING_OBJECT_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for metadata_key in METADATA_KEYS {
        let Some(raw) = sanitized.get(metadata_key) else {
            continue;
        };
        if let Some(trimmed) = trim_markers(raw, own_marker)? {
            sanitized.insert(metadata_key.to_owned(), trimmed);
        }
    }
    Ok(sanitized)
}

// None leaves the metadata as it was: it is not a mapping, or holds no marker list
fn trim_markers(
    raw: &RawValue,
    own_marker: Option<&str>,
) -> Result<Option<Box<RawValue>>, RecoveryError> {
    let Ok(mut metadata) = serde_json::from_str::<IndexMap<String, Box<RawValue>>>(raw.get())
    else {
        return Ok(None);
    };
    let Some(marker_list) = metadata.get(PRE_CALL_EXECUTED_KEY) else {
        return Ok(None);
    };
    let Ok(executed) = serde_json::from_str::<Vec<JsonValue>>(marker_list.get()) else {
        return Ok(None);
    };
    let kept: Vec<JsonValue> = executed
        .into_iter()
        .filter(
            |marker| matches!(marker, JsonValue::String(text) if Some(text.as_str()) == own_marker),
        )
        .collect();
    if kept.is_empty() {
        metadata.shift_remove(PRE_CALL_EXECUTED_KEY);
    } else {
        metadata.insert(PRE_CALL_EXECUTED_KEY.to_owned(), to_raw_value(&kept)?);
    }
    Ok(Some(to_raw_value(&metadata)?))
}
