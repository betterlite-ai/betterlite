use std::collections::BTreeMap;

use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use indexmap::IndexMap;

use super::error::WsNativeError;
use crate::api::responses_parameters::requested::requested_optional_params;

// Python truthiness over JSON: null, false, zero, "" and empty containers are false
#[must_use]
pub fn is_supplied(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

#[must_use]
pub(super) fn json_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) => {
            if number.is_f64() {
                "float"
            } else {
                "int"
            }
        }
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

// JSON that isn't an object fails the way Python's .get does; malformed JSON is simply not read
pub(super) fn json_object(message: &str) -> Result<Option<IndexMap<String, Json>>, WsNativeError> {
    match serde_json::from_str::<Json>(message) {
        Ok(Json::Object(fields)) => Ok(Some(fields)),
        Ok(other) => Err(WsNativeError::AttributeError {
            kind: json_type_name(&other),
        }),
        Err(_) => Ok(None),
    }
}

enum SlotError {
    TypeError,
    AttributeError,
}

// kwargs.get(name) or {}: a falsey value is an empty mapping, a truthy non-mapping raises
fn mapping_slot<'value>(
    value: Option<&'value Json>,
    error: SlotError,
) -> Result<Option<&'value IndexMap<String, Json>>, WsNativeError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(other) if is_supplied(other) => {
            let kind = json_type_name(other);
            Err(match error {
                SlotError::TypeError => WsNativeError::TypeError { kind },
                SlotError::AttributeError => WsNativeError::AttributeError { kind },
            })
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => Ok(None),
    }
}

// {**litellm_metadata}: a truthy non-mapping raises, and the guardrails prefer the caller's metadata
pub fn build_litellm_metadata_for_ws(
    kwargs: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, WsNativeError> {
    let mut metadata = mapping_slot(kwargs.get("litellm_metadata"), SlotError::TypeError)?
        .cloned()
        .unwrap_or_default();
    let caller_metadata = mapping_slot(kwargs.get("metadata"), SlotError::AttributeError)?;
    let guardrails = caller_metadata
        .and_then(|fields| fields.get("guardrails"))
        .filter(|value| is_supplied(value))
        .or_else(|| kwargs.get("guardrails").filter(|value| is_supplied(value)));
    if let Some(guardrails) = guardrails {
        metadata.insert(String::from("guardrails"), guardrails.clone());
    }
    Ok(metadata)
}

// a mapping effort is validated and kept, a scalar one is wrapped, and a present reasoning suppresses both
#[must_use]
pub fn deployment_reasoning_default(
    kwargs: &IndexMap<String, Json>,
    auto_summary: bool,
) -> Option<Json> {
    if kwargs
        .get("reasoning")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return None;
    }
    let effort = kwargs.get("reasoning_effort")?;
    match effort {
        Json::Null => None,
        Json::Object(_) => Some(effort.clone()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Some(reasoning_effort_object(effort, auto_summary))
        }
    }
}

fn reasoning_effort_object(effort: &Json, auto_summary: bool) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("effort"), effort.clone());
    if auto_summary {
        fields.insert(
            String::from("summary"),
            Json::Text(String::from("detailed")),
        );
    }
    Json::Object(fields)
}

// the routed input replaces a nested response input when one exists, otherwise the flat input
#[must_use]
pub fn routed_first_frame(first_message: &str, routed_input: Option<&Json>) -> String {
    let Ok(Json::Object(frame)) = serde_json::from_str::<Json>(first_message) else {
        return first_message.to_owned();
    };
    let Some(routed) = routed_input else {
        return first_message.to_owned();
    };
    if let Some(Json::Object(nested)) = frame.get("response")
        && let Some(current) = nested.get("input")
        && !matches!(current, Json::Null)
    {
        if current == routed {
            return first_message.to_owned();
        }
        return rewrite_nested(&frame, nested, routed);
    }
    if frame.get("input") == Some(routed) {
        return first_message.to_owned();
    }
    rewrite_flat(&frame, routed)
}

fn rewrite_nested(
    frame: &IndexMap<String, Json>,
    nested: &IndexMap<String, Json>,
    routed: &Json,
) -> String {
    let mut updated_nested = nested.clone();
    updated_nested.insert(String::from("input"), routed.clone());
    let mut updated_frame = frame.clone();
    updated_frame.insert(String::from("response"), Json::Object(updated_nested));
    serde_json::to_string(&Json::Object(updated_frame)).unwrap_or_default()
}

fn rewrite_flat(frame: &IndexMap<String, Json>, routed: &Json) -> String {
    let mut updated_frame = frame.clone();
    updated_frame.insert(String::from("input"), routed.clone());
    serde_json::to_string(&Json::Object(updated_frame)).unwrap_or_default()
}

pub fn ws_request_defaults(
    deployment_kwargs: &IndexMap<String, Json>,
    auto_summary: bool,
) -> Result<WsRequestDefaults<Json>, WsNativeError> {
    let mut candidate = deployment_kwargs.clone();
    if let Some(reasoning) = deployment_reasoning_default(deployment_kwargs, auto_summary) {
        candidate.insert(String::from("reasoning"), reasoning);
    }
    let mut params: Params =
        serde_json::from_str(&serde_json::to_string(&Json::Object(candidate))?)?;
    let fill_missing = serde_json::from_str::<BTreeMap<String, Json>>(&serde_json::to_string(
        &requested_optional_params(&mut params)?,
    )?)?;
    let overrides = extra_body_overrides(deployment_kwargs.get("extra_body"))?;
    Ok(WsRequestDefaults {
        fill_missing,
        overrides,
    })
}

fn extra_body_overrides(value: Option<&Json>) -> Result<BTreeMap<String, Json>, WsNativeError> {
    match value {
        None | Some(Json::Null) => Ok(BTreeMap::new()),
        Some(Json::Object(fields)) => Ok(fields.clone().into_iter().collect()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(WsNativeError::Validation {
                message: String::from("extra_body must be a JSON object"),
            })
        }
    }
}
