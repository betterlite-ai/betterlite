use betterlite_logging::spend_bodies::messages::storage_messages;
use betterlite_logging::spend_bodies::support::NodeAllocator;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::value::SpendValue;
use serde_json::{Value, json};

use super::pipeline::{Renderer, optional_decode, request_text, response_text, storage_policy_of};
use super::support::{decode, mapping_of};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyStorageSpec {
    store_bodies: bool,
    max_string_length: i64,
    turn_off_message_logging: bool,
    litellm_params: Value,
    standard_logging_object: Option<Value>,
    realtime_tools: Option<Value>,
    dynamic_params: Option<Value>,
    response: Value,
    realtime_tool_calls: Option<Value>,
    messages_payload: Option<Value>,
}

pub(super) fn body_storage(spec: &BodyStorageSpec) -> crate::helpers::HelperResult {
    let mut nodes = NodeAllocator::default();
    let mut render = Renderer {
        policy: storage_policy_of(
            SpendSanitizePolicy {
                store_bodies: spec.store_bodies,
                max_string_length: spec.max_string_length,
            },
            spec.turn_off_message_logging,
        ),
        nodes: &mut nodes,
    };
    let params = decode(&spec.litellm_params, render.nodes)?;
    let standard = optional_decode(spec.standard_logging_object.as_ref(), render.nodes)?;
    let tools = optional_decode(spec.realtime_tools.as_ref(), render.nodes)?;
    let dynamic = optional_decode(spec.dynamic_params.as_ref(), render.nodes)?;
    let stored_response = decode(&spec.response, render.nodes)?;
    let tool_calls = optional_decode(spec.realtime_tool_calls.as_ref(), render.nodes)?;
    let stored_messages = optional_decode(spec.messages_payload.as_ref(), render.nodes)?;
    let request = request_text(
        &mut render,
        &params,
        standard.as_ref(),
        tools.as_ref(),
        dynamic.as_ref(),
    )?;
    let response = response_text(
        &mut render,
        &stored_response,
        tool_calls.as_ref(),
        Some(&params),
        dynamic.as_ref(),
    );
    let messages = messages_text(stored_messages.as_ref(), spec.store_bodies)?;
    Ok(json!({
        "request": request,
        "response": response,
        "messages": messages,
    }))
}

fn messages_text(payload: Option<&SpendValue>, store_bodies: bool) -> crate::helpers::HelperResult {
    match payload {
        Some(stored) => Ok(Value::String(storage_messages(
            Some(mapping_of(stored)?),
            store_bodies,
        ))),
        None => Ok(Value::String("{}".to_owned())),
    }
}
