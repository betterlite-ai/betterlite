use std::collections::BTreeMap;

use betterlite_streaming::stream_tool_assembly::records::{
    FunctionOutput, JsonValue as ToolJsonValue, ToolOutput,
};
use betterlite_streaming::streamed_response_assembly::response::ChatChoice;
use serde::Serialize;

#[derive(Serialize)]
struct FunctionJson<'call> {
    arguments: &'call str,
    name: Option<&'call str>,
}

#[derive(Serialize)]
struct ProviderToolJson<'call> {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_specific_fields: Option<&'call BTreeMap<String, ToolJsonValue>>,
    function: FunctionJson<'call>,
    id: &'call str,
    r#type: &'call str,
}

#[derive(Serialize)]
struct CustomToolJson<'call> {
    id: &'call str,
    r#type: &'call str,
    custom: CustomJson<'call>,
}

#[derive(Serialize)]
struct CustomJson<'call> {
    name: &'call str,
    input: &'call str,
}

// B218 get_content_from_model_response: each choice's text, then its function call, then every tool call.
pub(super) fn completion_output(choices: &[ChatChoice]) -> Result<String, serde_json::Error> {
    let mut output = String::new();
    for choice in choices {
        output.push_str(choice.message.content.as_deref().unwrap_or_default());
        if let Some(call) = &choice.message.function_call {
            output.push_str(&function_json(call)?);
        }
        for call in choice.message.tool_calls.iter().flatten() {
            output.push_str(&tool_json(call)?);
        }
    }
    Ok(output)
}

fn function_json(call: &FunctionOutput) -> Result<String, serde_json::Error> {
    serde_json::to_string(&FunctionJson {
        arguments: &call.arguments,
        name: call.name.as_deref(),
    })
}

// The keyword order is the order ChatCompletionMessageToolCall.model_dump_json writes.
fn tool_json(call: &ToolOutput) -> Result<String, serde_json::Error> {
    if let Some(custom) = &call.custom {
        return serde_json::to_string(&CustomToolJson {
            id: &call.id,
            r#type: &call.kind,
            custom: CustomJson {
                name: &custom.name,
                input: &custom.input,
            },
        });
    }
    let Some(function) = call.function.as_ref() else {
        return Ok(String::new());
    };
    serde_json::to_string(&ProviderToolJson {
        provider_specific_fields: call.provider_specific_fields.as_ref(),
        function: FunctionJson {
            arguments: &function.arguments,
            name: function.name.as_deref(),
        },
        id: &call.id,
        r#type: &call.kind,
    })
}
