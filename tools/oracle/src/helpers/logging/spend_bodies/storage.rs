use betterlite_logging::spend_bodies::response::{ResponseKwargs, storage_response};
use betterlite_logging::spend_bodies::support::NodeAllocator;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::value::{SpendMapping, SpendValue};
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::pipeline::{Renderer, messages_text, optional_decode, request_text, storage_policy_of};
use super::support::decode;

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

struct DecodedBody {
    params: SpendValue,
    standard: Option<SpendValue>,
    tools: Option<SpendValue>,
    dynamic: Option<SpendValue>,
    response: SpendValue,
    tool_calls: Option<SpendValue>,
    messages: Option<SpendValue>,
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
    let decoded = decode_body(spec, render.nodes)?;
    let payload = SpendMapping {
        node: render.nodes.allocate(),
        entries: IndexMap::from([("response".to_owned(), decoded.response)]),
    };
    let kwargs = ResponseKwargs {
        realtime_tool_calls: decoded.tool_calls.as_ref(),
        litellm_params: Some(&decoded.params),
        dynamic_params: decoded.dynamic.as_ref(),
    };
    Ok(json!({
        "request": request_text(&mut render, &decoded.params, decoded.standard.as_ref(), decoded.tools.as_ref(), decoded.dynamic.as_ref())?,
        "response": storage_response(Some(&payload), Some(&kwargs), render.policy, render.nodes),
        "messages": messages_text(decoded.messages.as_ref(), spec.store_bodies)?,
    }))
}

fn decode_body(
    spec: &BodyStorageSpec,
    nodes: &mut NodeAllocator,
) -> Result<DecodedBody, Box<crate::helpers::HelperError>> {
    Ok(DecodedBody {
        params: decode(&spec.litellm_params, nodes)?,
        standard: optional_decode(spec.standard_logging_object.as_ref(), nodes)?,
        tools: optional_decode(spec.realtime_tools.as_ref(), nodes)?,
        dynamic: optional_decode(spec.dynamic_params.as_ref(), nodes)?,
        response: decode(&spec.response, nodes)?,
        tool_calls: optional_decode(spec.realtime_tool_calls.as_ref(), nodes)?,
        messages: optional_decode(spec.messages_payload.as_ref(), nodes)?,
    })
}
