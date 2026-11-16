use std::collections::BTreeMap;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

#[path = "content_projections/request_io/tests.rs"]
mod request_io;
#[path = "content_projections/stream_output/tests.rs"]
mod stream_output;

// the production assembler is built over B3669 and B2227 in crates that depend on this one
pub trait StreamOutputAssembler: Send + Sync {
    fn assemble_sse(&self, frames: &[Vec<u8>]) -> Option<OtelModelValue>;

    fn assemble_chat(
        &self,
        chunks: &[OtelModelValue],
        messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue>;
}

#[derive(Clone, Debug)]
pub enum RelayedChunk {
    Raw(Vec<u8>),
    Chat(OtelModelValue),
    Completed(OtelModelValue),
    Other(OtelModelValue),
}

// LiteLLM tells the three response shapes apart by class, which JSON does not carry
#[derive(Clone, Copy, Debug)]
pub enum OutputResponse<'response> {
    ModelResponse(&'response OtelModelValue),
    ResponsesApi(&'response OtelModelValue),
    Other(&'response OtelModelValue),
}

fn turn(role: &str, content: OtelModelValue) -> OtelModelValue {
    OtelModelValue::Object(BTreeMap::from([
        ("content".to_owned(), content),
        ("role".to_owned(), OtelModelValue::String(role.to_owned())),
    ]))
}

fn system_turns(data: &MetadataMap) -> Vec<OtelModelValue> {
    ["system", "instructions"]
        .iter()
        .filter_map(|key| data.get(*key))
        .filter(|content| !matches!(content, OtelModelValue::Null(())))
        .map(|content| turn("system", content.clone()))
        .collect()
}

fn user_turns(turns: &OtelModelValue) -> Vec<OtelModelValue> {
    match turns {
        OtelModelValue::Array(items) => items.clone(),
        other @ (OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_)
        | OtelModelValue::Object(_)) => vec![turn("user", other.clone())],
    }
}

// a key present with a null value wins over input, like dict.get("messages", data.get("input"))
#[must_use]
pub fn request_input(data: &MetadataMap) -> Option<String> {
    let turns = if let Some(turns) = data.get("messages") {
        turns
    } else {
        data.get("input")?
    };
    if matches!(turns, OtelModelValue::Null(())) {
        return None;
    }
    let mut all = system_turns(data);
    all.extend(user_turns(turns));
    serde_json::to_string(&OtelModelValue::Array(all)).ok()
}

// pydantic's exclude_none drops null fields at every depth of a model, and keeps list items
fn without_nulls(value: &OtelModelValue) -> OtelModelValue {
    match value {
        OtelModelValue::Object(fields) => OtelModelValue::Object(
            fields
                .iter()
                .filter(|(_, item)| !matches!(item, OtelModelValue::Null(())))
                .map(|(name, item)| (name.clone(), without_nulls(item)))
                .collect(),
        ),
        OtelModelValue::Array(items) => {
            OtelModelValue::Array(items.iter().map(without_nulls).collect())
        }
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_) => value.clone(),
    }
}

fn chat_message(choice: &OtelModelValue) -> OtelModelValue {
    let mut message = choice
        .as_object()
        .and_then(|fields| fields.get("message"))
        .and_then(OtelModelValue::as_object)
        .cloned()
        .unwrap_or_default();
    message
        .entry("role".to_owned())
        .or_insert_with(|| OtelModelValue::String("assistant".to_owned()));
    without_nulls(&OtelModelValue::Object(message))
}

fn model_response_output(response: &OtelModelValue) -> Option<String> {
    let messages: Vec<OtelModelValue> = match response.as_object()?.get("choices") {
        Some(OtelModelValue::Array(choices)) => choices.iter().map(chat_message).collect(),
        None | Some(OtelModelValue::Null(())) => vec![chat_message(&OtelModelValue::Null(()))],
        Some(
            OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Object(_),
        ) => return None,
    };
    serde_json::to_string(&OtelModelValue::Array(messages)).ok()
}

fn responses_output(response: &OtelModelValue) -> Option<String> {
    let output = response
        .as_object()?
        .get("output")
        .map_or(OtelModelValue::Null(()), without_nulls);
    serde_json::to_string(&output).ok()
}

// the Anthropic message model needs type "message", a string role (default assistant) and any content
fn anthropic_message_output(message: &OtelModelValue) -> Option<String> {
    let fields = message.as_object()?;
    if fields.get("type").and_then(OtelModelValue::as_str) != Some("message") {
        return None;
    }
    let role = match fields.get("role") {
        None => "assistant",
        Some(OtelModelValue::String(text)) => text.as_str(),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        ) => return None,
    };
    let content = fields
        .get("content")
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    let dumped = OtelModelValue::Object(BTreeMap::from([
        ("content".to_owned(), content),
        ("role".to_owned(), OtelModelValue::String(role.to_owned())),
    ]));
    serde_json::to_string(&OtelModelValue::Array(vec![dumped])).ok()
}

#[must_use]
pub fn response_output(response: &OutputResponse<'_>) -> Option<String> {
    match response {
        OutputResponse::ModelResponse(value) => model_response_output(value),
        OutputResponse::ResponsesApi(value) => responses_output(value),
        OutputResponse::Other(value) => anthropic_message_output(value),
    }
}

fn raw_frames(chunks: &[RelayedChunk]) -> Vec<Vec<u8>> {
    chunks
        .iter()
        .filter_map(|chunk| match chunk {
            RelayedChunk::Raw(frame) => Some(frame.clone()),
            RelayedChunk::Chat(_) | RelayedChunk::Completed(_) | RelayedChunk::Other(_) => None,
        })
        .collect()
}

fn chat_chunks(chunks: &[RelayedChunk]) -> Option<Vec<OtelModelValue>> {
    chunks
        .iter()
        .map(|chunk| match chunk {
            RelayedChunk::Chat(value) => Some(value.clone()),
            RelayedChunk::Raw(_) | RelayedChunk::Completed(_) | RelayedChunk::Other(_) => None,
        })
        .collect()
}

fn assembled_chat_stream(
    values: &[OtelModelValue],
    data: &MetadataMap,
    assembler: &dyn StreamOutputAssembler,
) -> Option<OtelModelValue> {
    match data.get("messages") {
        None | Some(OtelModelValue::Null(())) => assembler.assemble_chat(values, None),
        Some(OtelModelValue::Array(items)) => {
            assembler.assemble_chat(values, Some(items.as_slice()))
        }
        Some(
            OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Object(_),
        ) => None,
    }
}

#[must_use]
pub fn stream_output(
    chunks: &[RelayedChunk],
    data: &MetadataMap,
    assembler: &dyn StreamOutputAssembler,
) -> Option<String> {
    if chunks.is_empty() {
        return None;
    }
    if chunks
        .iter()
        .any(|chunk| matches!(chunk, RelayedChunk::Raw(_)))
    {
        let assembled = assembler.assemble_sse(&raw_frames(chunks))?;
        return response_output(&OutputResponse::ModelResponse(&assembled));
    }
    if let Some(values) = chat_chunks(chunks) {
        let assembled = assembled_chat_stream(&values, data, assembler)?;
        return response_output(&OutputResponse::ModelResponse(&assembled));
    }
    let completed = chunks.iter().rev().find_map(|chunk| match chunk {
        RelayedChunk::Completed(response) => Some(response),
        RelayedChunk::Raw(_) | RelayedChunk::Chat(_) | RelayedChunk::Other(_) => None,
    })?;
    response_output(&OutputResponse::ResponsesApi(completed))
}
