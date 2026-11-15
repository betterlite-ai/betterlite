use std::collections::BTreeMap;

use serde::Serialize;

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_tool_assembly::records::{
    FunctionOutput, JsonValue as ToolJsonValue, ToolOutput,
};

use super::records::Choice;

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

// B218 get_content_from_model_response: the message text, then the function call, then each tool call.
pub fn completion_output(
    content: Option<&str>,
    function_call: Option<&FunctionOutput>,
    tool_calls: Option<&[ToolOutput]>,
) -> Result<String, serde_json::Error> {
    let mut output = content.unwrap_or_default().to_owned();
    if let Some(call) = function_call {
        output.push_str(&serde_json::to_string(&FunctionJson {
            arguments: &call.arguments,
            name: call.name.as_deref(),
        })?);
    }
    for call in tool_calls.unwrap_or_default() {
        output.push_str(&tool_json(call)?);
    }
    Ok(output)
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

// provider_fields_citations: generated citations first, then every first-choice map in stream order.
#[must_use]
pub fn combined_provider_fields(choices: &[&Choice]) -> Option<JsonObject> {
    let maps: Vec<&JsonObject> = choices
        .iter()
        .filter_map(|choice| choice.provider_fields())
        .collect();
    // A null citation is skipped like an absent one, since LiteLLM tests fields.get("citation") is not None.
    let citations: Vec<&JsonValue> = maps
        .iter()
        .filter_map(|fields| match fields.get("citation") {
            Some(JsonValue::Null) | None => None,
            Some(citation) => Some(citation),
        })
        .collect();
    let mut combined: JsonObject = BTreeMap::new();
    if !citations.is_empty() {
        combined.insert("citations".to_owned(), joined_citations(&citations));
    }
    for fields in &maps {
        for (key, value) in *fields {
            if key != "citation" {
                combined.insert(key.clone(), value.clone());
            }
        }
    }
    (!combined.is_empty()).then_some(combined)
}

// All list-shaped citations keep their own lists; anything else is wrapped in one outer list.
fn joined_citations(citations: &[&JsonValue]) -> JsonValue {
    let values: Vec<JsonValue> = citations
        .iter()
        .map(|citation| (*citation).clone())
        .collect();
    if citations
        .iter()
        .all(|citation| matches!(citation, JsonValue::Array(_)))
    {
        return JsonValue::Array(values);
    }
    JsonValue::Array(vec![JsonValue::Array(values)])
}
