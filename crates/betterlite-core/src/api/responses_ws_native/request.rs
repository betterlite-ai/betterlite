use std::collections::BTreeMap;

use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use indexmap::IndexMap;

use super::error::WsNativeError;
use crate::api::response_chat_request::options::map_optional_params;
use crate::api::responses_parameters::requested::requested_optional_params;

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

fn mapping_slot<'json>(
    value: Option<&'json Json>,
    error: &SlotError,
) -> Result<Option<&'json IndexMap<String, Json>>, WsNativeError> {
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

pub fn build_litellm_metadata_for_ws(
    kwargs: &IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, WsNativeError> {
    let mut metadata = mapping_slot(kwargs.get("litellm_metadata"), &SlotError::TypeError)?
        .cloned()
        .unwrap_or_default();
    let caller_metadata = mapping_slot(kwargs.get("metadata"), &SlotError::AttributeError)?;
    let selected_guardrails = caller_metadata
        .and_then(|fields| fields.get("guardrails"))
        .filter(|value| is_supplied(value))
        .or_else(|| kwargs.get("guardrails").filter(|value| is_supplied(value)));
    if let Some(guardrails) = selected_guardrails {
        metadata.insert(String::from("guardrails"), guardrails.clone());
    }
    Ok(metadata)
}

pub fn deployment_reasoning_default(
    kwargs: &IndexMap<String, Json>,
    auto_summary: bool,
) -> Result<Option<Json>, WsNativeError> {
    if kwargs
        .get("reasoning")
        .is_some_and(|value| !matches!(value, Json::Null))
    {
        return Ok(None);
    }
    let Some(effort) = kwargs.get("reasoning_effort") else {
        return Ok(None);
    };
    let optional = IndexMap::from([(String::from("reasoning_effort"), effort.clone())]);
    let mut params = serde_json::from_str("{}")?;
    map_optional_params(&optional, &mut params, auto_summary)?;
    let Some(reasoning) = params.remove("reasoning") else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(reasoning)?))
}

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
        let mut updated_nested = nested.clone();
        updated_nested.insert(String::from("input"), routed.clone());
        let mut updated = frame.clone();
        updated.insert(String::from("response"), Json::Object(updated_nested));
        return serde_json::to_string(&Json::Object(updated)).unwrap_or_default();
    }
    if frame.get("input") == Some(routed) {
        return first_message.to_owned();
    }
    let mut updated = frame;
    updated.insert(String::from("input"), routed.clone());
    serde_json::to_string(&Json::Object(updated)).unwrap_or_default()
}

pub fn ws_request_defaults(
    deployment_kwargs: &IndexMap<String, Json>,
    auto_summary: bool,
) -> Result<WsRequestDefaults<Json>, WsNativeError> {
    let mut candidate = deployment_kwargs.clone();
    if let Some(reasoning) = deployment_reasoning_default(deployment_kwargs, auto_summary)? {
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
    let filtered = value.filter(|candidate| !matches!(**candidate, Json::Null));
    let Some(present) = filtered else {
        return Ok(BTreeMap::new());
    };
    let Json::Object(fields) = present else {
        return Err(WsNativeError::Validation {
            message: String::from("extra_body must be a JSON object"),
        });
    };
    Ok(fields.clone().into_iter().collect())
}
