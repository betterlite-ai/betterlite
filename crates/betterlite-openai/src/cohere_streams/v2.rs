use serde_json::{Map, Value};

use super::{CohereChunk, CohereStreamError, CohereUsage, int_value};

pub(super) fn parse_v2(value: &Value) -> Result<CohereChunk, CohereStreamError> {
    let index = int_value(value.get("index"))?;
    let chunk_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let event = value
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut chunk = empty_chunk(index);
    match chunk_type {
        "content-delta" => parse_content_delta(value, &mut chunk),
        "tool-call-delta" => parse_tool_call_delta(value, &mut chunk),
        _ => parse_event(event, value, &mut chunk),
    }
    copy_top_level_citations(value, &mut chunk);
    Ok(chunk)
}

pub(super) const fn empty_chunk(index: i64) -> CohereChunk {
    CohereChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index,
        provider_specific_fields: None,
    }
}

fn parse_content_delta(value: &Value, chunk: &mut CohereChunk) {
    let content = value.pointer("/delta/message/content");
    content
        .and_then(Value::as_str)
        .or_else(|| {
            content
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .clone_into(&mut chunk.text);
}

fn parse_tool_call_delta(value: &Value, chunk: &mut CohereChunk) {
    let first_tool = value
        .pointer("/delta/tool_calls")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    chunk.tool_use = first_tool.map(tool_call);
}

fn tool_call(tool: &Value) -> Value {
    let function = Map::from_iter([
        (String::from("name"), string_field(tool, "name")),
        (String::from("arguments"), string_field(tool, "arguments")),
    ]);
    Value::Object(Map::from_iter([
        (String::from("id"), string_field(tool, "id")),
        (
            String::from("type"),
            Value::String(String::from("function")),
        ),
        (String::from("function"), Value::Object(function)),
    ]))
}

fn parse_event(event: &str, value: &Value, chunk: &mut CohereChunk) {
    match event {
        "tool-plan-delta" => parse_tool_plan(value, chunk),
        "citation-start" => parse_citation_start(value, chunk),
        "message-end" => parse_message_end(value, chunk),
        _ => {}
    }
}

fn parse_tool_plan(value: &Value, chunk: &mut CohereChunk) {
    chunk.provider_specific_fields = value
        .pointer("/data/delta/message/tool_plan")
        .filter(|item| !item.is_null() && item.as_str() != Some(""))
        .map(|plan| Value::Object(Map::from_iter([(String::from("tool_plan"), plan.clone())])));
}

fn parse_citation_start(value: &Value, chunk: &mut CohereChunk) {
    chunk.provider_specific_fields = value
        .pointer("/data/delta/message/citations")
        .filter(|item| item.is_object() && !item.as_object().is_some_and(Map::is_empty))
        .map(citation_fields);
}

fn citation_fields(citations: &Value) -> Value {
    let citation = Map::from_iter([
        (
            String::from("start"),
            citations
                .get("start")
                .map_or_else(|| Value::from(0_u64), Clone::clone),
        ),
        (
            String::from("end"),
            citations
                .get("end")
                .map_or_else(|| Value::from(0_u64), Clone::clone),
        ),
        (
            String::from("text"),
            citations
                .get("text")
                .map_or_else(|| Value::String(String::new()), Clone::clone),
        ),
        (
            String::from("sources"),
            citations
                .get("sources")
                .map_or_else(|| Value::Array(Vec::new()), Clone::clone),
        ),
        (
            String::from("type"),
            citations
                .get("type")
                .map_or_else(|| Value::String(String::from("TEXT_CONTENT")), Clone::clone),
        ),
    ]);
    Value::Object(Map::from_iter([(
        String::from("citations"),
        Value::Array(vec![Value::Object(citation)]),
    )]))
}

fn parse_message_end(value: &Value, chunk: &mut CohereChunk) {
    let delta = value.get("data").and_then(|data| data.get("delta"));
    chunk.is_finished = true;
    delta
        .and_then(|data| data.get("finish_reason"))
        .and_then(Value::as_str)
        .unwrap_or("stop")
        .clone_into(&mut chunk.finish_reason);
    chunk.usage = delta.and_then(usage_from_delta);
}

fn usage_from_delta(delta: &Value) -> Option<CohereUsage> {
    let usage = delta.get("usage")?;
    if usage.is_null() || usage.as_object().is_some_and(Map::is_empty) {
        return None;
    }
    let tokens = usage.get("tokens")?;
    let prompt = tokens
        .get("input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion = tokens
        .get("output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some(CohereUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
    })
}

fn string_field(value: &Value, field: &str) -> Value {
    Value::String(
        value
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    )
}

fn copy_top_level_citations(value: &Value, chunk: &mut CohereChunk) {
    if let Some(citations) = value.get("citations") {
        let provider_fields = chunk
            .provider_specific_fields
            .get_or_insert_with(|| Value::Object(Map::new()));
        if let Some(object) = provider_fields.as_object_mut() {
            object.insert(String::from("citations"), citations.clone());
        }
    }
}
