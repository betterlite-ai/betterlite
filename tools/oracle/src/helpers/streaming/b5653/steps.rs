use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, RequestOptions, as_object,
};
use betterlite_streaming::provider_chunk_decode::creator::chunk_creator;
use betterlite_streaming::provider_chunk_decode::dispatch::decode_provider_chunk;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use betterlite_streaming::provider_chunk_decode::handlers::process_chunk;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{
    GenericChunk, JsonObject, JsonValue, StreamChunk, Usage,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::HelperError;

struct Harness {
    provider: Option<String>,
    state: DecodeState,
    shaper: ChunkShaper,
}

pub(super) fn run(input: &Value) -> Result<Value, Box<HelperError>> {
    let spec = input.get("state").cloned().unwrap_or_default();
    let mut harness = Harness {
        provider: load(&spec, "provider")?,
        state: state_of(&spec)?,
        shaper: shaper_of(&spec)?,
    };
    let steps: Vec<Value> = load(input, "steps")?.unwrap_or_default();
    let mut outputs = Vec::with_capacity(steps.len());
    for step in &steps {
        outputs.push(apply(&mut harness, step)?);
    }
    Ok(Value::Array(outputs))
}

fn load<T>(value: &Value, key: &str) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(raw) = value.get(key).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone()).map_err(|error| invalid(error.to_string()))
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn state_of(spec: &Value) -> Result<DecodeState, Box<HelperError>> {
    let kind = if load::<bool>(spec, "custom_provider")?.unwrap_or(false) {
        ProviderKind::Custom
    } else {
        ProviderKind::Builtin
    };
    Ok(DecodeState {
        model: load::<String>(spec, "model")?.unwrap_or_default(),
        kind,
        received_finish_reason: load(spec, "received_finish_reason")?,
        intermittent_finish_reason: load(spec, "intermittent_finish_reason")?,
        sent_first_chunk: load::<bool>(spec, "sent_first_chunk")?.unwrap_or(false),
        complete_response: load::<String>(spec, "complete_response")?.unwrap_or_default(),
        completion_stream: load(spec, "completion_stream")?,
        options: RequestOptions {
            send_stream_usage: load::<bool>(spec, "send_stream_usage")?.unwrap_or(false),
            is_function_call: load::<bool>(spec, "is_function_call")?.unwrap_or(false),
        },
        repeated_chunk_limit: load::<usize>(spec, "repeated_chunk_limit")?.unwrap_or(100),
    })
}

fn shaper_of(spec: &Value) -> Result<ChunkShaper, Box<HelperError>> {
    let mut shaper = ChunkShaper::new(
        load::<String>(spec, "model")?.unwrap_or_default(),
        load(spec, "provider")?,
        load::<bool>(spec, "merge_reasoning_in_choices")?.unwrap_or(false),
        load::<JsonObject>(spec, "base_hidden_params")?.unwrap_or_default(),
    );
    shaper.response_id = load(spec, "response_id")?;
    shaper.created = load(spec, "created")?;
    shaper.system_fingerprint = load(spec, "system_fingerprint")?;
    shaper.chunks = load::<Vec<StreamChunk>>(spec, "chunks")?.unwrap_or_default();
    shaper.progress.sent_first_chunk = load::<bool>(spec, "sent_first_chunk")?.unwrap_or(false);
    shaper.tool_call = load::<bool>(spec, "tool_call")?.unwrap_or(false);
    shaper.received_finish_reason = load(spec, "received_finish_reason")?;
    shaper.intermittent_finish_reason = load(spec, "intermittent_finish_reason")?;
    shaper.now = fixed_instant;
    shaper.next_id = Uuid::nil;
    Ok(shaper)
}

const fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

fn apply(harness: &mut Harness, step: &Value) -> Result<Value, Box<HelperError>> {
    if let Some(raw) = step.get("provider") {
        let provider = raw.as_str().map(str::to_owned);
        harness.provider.clone_from(&provider);
        harness.shaper.provider = provider;
    }
    if let Some(model) = load::<String>(step, "model")? {
        harness.state.model.clone_from(&model);
        harness.shaper.model = model;
    }
    if let Some(usage) = load::<bool>(step, "send_stream_usage")? {
        harness.state.options.send_stream_usage = usage;
    }
    if let Some(is_call) = load::<bool>(step, "is_function_call")? {
        harness.state.options.is_function_call = is_call;
    }
    let op: String = load(step, "op")?.unwrap_or_default();
    let chunk = chunk_of(step)?;
    match op.as_str() {
        "decode" => {
            match decode_provider_chunk(harness.provider.as_deref(), &chunk, &mut harness.state) {
                Ok(decoded) => Ok(decode_view(&decoded, &harness.state)),
                Err(error) => Ok(raised(&error)),
            }
        }
        "creator" => match chunk_creator(&mut harness.shaper, &mut harness.state, &chunk) {
            Ok(created) => Ok(create_view(
                created.as_ref(),
                &harness.shaper,
                &harness.state,
            )),
            Err(error) => Ok(raised(&error)),
        },
        "process_chunk" => {
            let text: String = load(step, "chunk")?.unwrap_or_default();
            let processed = process_chunk(&mut harness.state, &text);
            Ok(json!([processed, harness.state.complete_response]))
        }
        other => Err(invalid(format!("unsupported operation {other}"))),
    }
}

fn chunk_of(step: &Value) -> Result<ProviderChunk, Box<HelperError>> {
    let spec = step.get("chunk").cloned().unwrap_or_default();
    let kind: String = load(&spec, "kind")?.unwrap_or_default();
    let value = spec.get("value").cloned().unwrap_or(Value::Null);
    let parsed = |error: serde_json::Error| invalid(error.to_string());
    match kind.as_str() {
        "text" => Ok(ProviderChunk::Text(
            load(&spec, "value")?.unwrap_or_default(),
        )),
        "bytes" => Ok(ProviderChunk::Bytes(
            load(&spec, "value")?.unwrap_or_default(),
        )),
        "generic" => Ok(ProviderChunk::Generic(Box::new(
            serde_json::from_value::<GenericChunk>(value).map_err(parsed)?,
        ))),
        "json" => Ok(ProviderChunk::Json(Box::new(
            serde_json::from_value::<JsonValue>(value).map_err(parsed)?,
        ))),
        other => Err(invalid(format!("unsupported chunk kind {other}"))),
    }
}

fn raised(error: &ProviderChunkError) -> Value {
    match error {
        ProviderChunkError::StopIteration => json!({"raised": {"message": ""}}),
        ProviderChunkError::ParseResponse { .. }
        | ProviderChunkError::RejectedResponse { .. }
        | ProviderChunkError::ProviderReported { .. }
        | ProviderChunkError::UnknownStream
        | ProviderChunkError::VertexBlocked
        | ProviderChunkError::NotAString { .. }
        | ProviderChunkError::UnexpectedChunk { .. }
        | ProviderChunkError::MissingField { .. }
        | ProviderChunkError::InvalidField { .. }
        | ProviderChunkError::InvalidUtf8 { .. }
        | ProviderChunkError::StreamingFailure { .. }
        | ProviderChunkError::InvalidChunk { .. }
        | ProviderChunkError::Shape { .. } => json!({"raised": {"message": error.to_string()}}),
    }
}

fn decode_view(decoded: &DecodedChunk, state: &DecodeState) -> Value {
    let reason = decoded
        .finish_reason
        .clone()
        .filter(|text| !text.is_empty());
    let usage = decoded.usage.as_ref().map(usage_tuple);
    let early_id = decoded.original.as_ref().map(|chunk| chunk.id.clone());
    to_array((
        &decoded.text,
        decoded.is_finished,
        reason,
        usage,
        &decoded.tool_use,
        &decoded.provider_specific_fields,
        &decoded.tool_calls,
        decoded.early_return,
        early_id,
        state_line(state),
    ))
}

fn state_line(state: &DecodeState) -> Value {
    to_array((
        &state.model,
        &state.received_finish_reason,
        &state.intermittent_finish_reason,
        &state.completion_stream,
    ))
}

fn create_view(chunk: Option<&StreamChunk>, shaper: &ChunkShaper, state: &DecodeState) -> Value {
    to_array((
        chunk.map(summary),
        &state.received_finish_reason,
        &state.intermittent_finish_reason,
        shaper.progress.sent_first_chunk,
        shaper.tool_call,
        shaper.chunks.len(),
    ))
}

fn summary(chunk: &StreamChunk) -> Value {
    let choice = chunk.choices.first();
    let body = choice.map(|first| &first.delta);
    let function = body
        .and_then(|part| part.function_call.as_ref())
        .and_then(as_object)
        .map(|fields| json!([fields.get("name"), fields.get("arguments")]));
    let calls = body.and_then(|part| part.tool_calls.as_ref());
    to_array((
        &chunk.id,
        &chunk.model,
        chunk.created,
        &chunk.system_fingerprint,
        choice.and_then(|first| first.finish_reason.clone()),
        body.and_then(|part| part.content.clone()),
        body.and_then(|part| part.role.clone()),
        function,
        calls.and_then(call_list),
        chunk.choices.len(),
        chunk.usage.as_ref().map(usage_tuple),
        &chunk.provider_specific_fields,
    ))
}

fn call_list(value: &JsonValue) -> Option<Value> {
    let JsonValue::Array(items) = value else {
        return None;
    };
    Some(Value::Array(items.iter().map(call_view).collect()))
}

fn call_view(value: &JsonValue) -> Value {
    let Some(fields) = as_object(value) else {
        return Value::Null;
    };
    let function = fields
        .get("function")
        .and_then(as_object)
        .map(|found| json!([found.get("name"), found.get("arguments")]));
    json!([fields.get("id"), fields.get("type"), function])
}

const fn usage_tuple(usage: &Usage) -> [i64; 3] {
    [
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
    ]
}

fn to_array<Plain>(value: Plain) -> Value
where
    Plain: Serialize,
{
    let Ok(plain) = serde_json::to_value(value) else {
        return Value::Null;
    };
    plain
}
