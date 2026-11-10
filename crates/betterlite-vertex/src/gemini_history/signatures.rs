use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::signatures::thought_signature_from_tool;
use betterlite_prompts::gemini_tool_invokes::types::GeminiTool;
use betterlite_prompts::message_content::Message;

const TOOL_CALLS: &str = "tool_calls";
const FUNCTION_CALL: &str = "function_call";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const SERVER_INVOCATIONS: &str = "server_side_tool_invocations";
const THOUGHT_SIGNATURE: &str = "thought_signature";
const RESPONSE_THOUGHT_SIGNATURE: &str = "response_thought_signature";
const FUNCTION_KEY: &str = "function";

#[must_use]
pub fn collect_tool_signatures(message: &Message) -> BTreeSet<String> {
    let mut signatures = BTreeSet::new();
    if let Some(Json::List(tools)) = message.provider_specific_fields.get(TOOL_CALLS) {
        for tool in tools {
            add_tool_signature(tool, &mut signatures);
        }
    }
    if let Some(function_call) = message.provider_specific_fields.get(FUNCTION_CALL) {
        let wrapped = Json::Object(Object::from_iter([(
            FUNCTION_KEY.to_owned(),
            function_call.clone(),
        )]));
        add_tool_signature(&wrapped, &mut signatures);
    }
    if let Some(Json::Object(provider_fields)) =
        message.provider_specific_fields.get(PROVIDER_FIELDS)
        && let Some(Json::List(invocations)) = provider_fields.get(SERVER_INVOCATIONS)
    {
        for invocation in invocations {
            add_invocation_signatures(invocation, &mut signatures);
        }
    }
    signatures
}

fn add_tool_signature(tool: &Json, signatures: &mut BTreeSet<String>) {
    let Ok(encoded) = serde_json::to_value(tool) else {
        return;
    };
    let Ok(decoded) = serde_json::from_value::<GeminiTool>(encoded) else {
        return;
    };
    if let Some(signature) = thought_signature_from_tool(&decoded) {
        signatures.insert(signature);
    }
}

fn add_invocation_signatures(invocation: &Json, signatures: &mut BTreeSet<String>) {
    let Some(fields) = invocation.as_object() else {
        return;
    };
    for name in [THOUGHT_SIGNATURE, RESPONSE_THOUGHT_SIGNATURE] {
        if let Some(Json::Text(signature)) = fields.get(name)
            && !signature.is_empty()
        {
            signatures.insert(signature.clone());
        }
    }
}
