use std::collections::BTreeMap;

use betterlite_openai::chat::{FunctionCall, ToolCall};
use betterlite_prompts::arguments::json::Json;

use crate::oci_chat_wire::ids::synthesize_oci_tool_call_id;
use crate::oci_chat_wire::types::cohere::CohereToolCall;

pub(crate) struct AdaptedCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

pub(crate) fn adapt_cohere_tool_calls(calls: &[CohereToolCall]) -> Vec<ToolCall> {
    calls
        .iter()
        .enumerate()
        .map(|(position, call)| {
            let adapted = adapt_call(position, call);
            ToolCall {
                id: adapted.id,
                kind: String::from("function"),
                function: FunctionCall {
                    name: adapted.name,
                    arguments: adapted.arguments,
                },
            }
        })
        .collect()
}

pub(crate) fn adapt_call(position: usize, call: &CohereToolCall) -> AdaptedCall {
    let arguments = serde_json::to_string(&call.parameters).unwrap_or_default();
    let sorted: BTreeMap<&String, &Json> = call.parameters.iter().collect();
    let sorted_text = serde_json::to_string(&sorted).unwrap_or_default();
    AdaptedCall {
        id: synthesize_oci_tool_call_id(position, &call.name, &sorted_text),
        name: call.name.clone(),
        arguments,
    }
}
