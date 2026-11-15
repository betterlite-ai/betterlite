use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::stream_base_assembly::records::Chunk as BaseChunk;
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_content_assembly::wire::Chunk as ContentWireChunk;
use crate::stream_tool_assembly::records::Chunk as ToolWireChunk;
use crate::stream_usage_assembly::wire::Chunk as UsageWireChunk;

use super::records::{Choice, Chunk};

// The shared typed records are the input to B2232/B2233/B2234/B2235, which each read their own fields.
pub(crate) fn project<T, S>(source: &S) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
    S: Serialize + ?Sized,
{
    serde_json::from_value(serde_json::to_value(source)?)
}

#[derive(Serialize)]
struct ContentProjection<'chunk> {
    choices: Vec<&'chunk Choice>,
}

#[derive(Serialize)]
struct ToolProjection<'chunk> {
    choices: Vec<ToolChoice<'chunk>>,
}

#[derive(Serialize)]
struct ToolChoice<'chunk> {
    index: Option<u32>,
    delta: ToolDelta<'chunk>,
}

#[derive(Serialize)]
struct ToolDelta<'chunk> {
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<&'chunk JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<&'chunk JsonValue>,
}

#[derive(Serialize)]
struct UsageProjection<'chunk> {
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<JsonValue>,
    #[serde(rename = "_hidden_params", skip_serializing_if = "Option::is_none")]
    hidden_params: Option<&'chunk JsonObject>,
    choices: Vec<UsageChoice<'chunk>>,
}

#[derive(Serialize)]
struct UsageChoice<'chunk> {
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<&'chunk str>,
}

pub(crate) fn content_chunks(
    chunks: &[&Chunk],
) -> Result<Vec<ContentWireChunk>, serde_json::Error> {
    chunks
        .iter()
        .map(|chunk| {
            project(&ContentProjection {
                choices: chunk.choices.iter().collect(),
            })
        })
        .collect()
}

pub(crate) fn tool_chunks(chunks: &[&Chunk]) -> Result<Vec<ToolWireChunk>, serde_json::Error> {
    chunks
        .iter()
        .map(|chunk| {
            project(&ToolProjection {
                choices: chunk
                    .choices
                    .iter()
                    .map(|choice| ToolChoice {
                        index: choice.index,
                        delta: ToolDelta {
                            tool_calls: choice
                                .delta
                                .as_ref()
                                .and_then(|delta| delta.tool_calls.as_ref()),
                            function_call: choice
                                .delta
                                .as_ref()
                                .and_then(|delta| delta.function_call.as_ref()),
                        },
                    })
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn base_chunks(chunks: &[Chunk]) -> Result<Vec<BaseChunk>, serde_json::Error> {
    chunks.iter().map(project).collect()
}

pub(crate) fn usage_chunks(chunks: &[Chunk]) -> Result<Vec<UsageWireChunk>, serde_json::Error> {
    chunks
        .iter()
        .map(|chunk| {
            project(&UsageProjection {
                usage: chunk.usage.as_ref().map(sanitized_usage),
                hidden_params: chunk.hidden_object(),
                choices: chunk
                    .choices
                    .iter()
                    .map(|choice| UsageChoice {
                        finish_reason: choice.finish_reason.as_deref(),
                    })
                    .collect(),
            })
        })
        .collect()
}

// Python's bool is an int, so a true or false cost is read as 1 or 0 before the Decimal record sees it.
fn sanitized_usage(usage: &JsonValue) -> JsonValue {
    let JsonValue::Object(fields) = usage else {
        return usage.clone();
    };
    let Some(JsonValue::Bool(flag)) = fields.get("cost") else {
        return usage.clone();
    };
    let mut sanitized = fields.clone();
    sanitized.insert(
        "cost".to_owned(),
        JsonValue::Number(serde_json::Number::from(u8::from(*flag))),
    );
    JsonValue::Object(sanitized)
}
