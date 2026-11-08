use betterlite_streaming::provider_chunk_decode::chunk::{
    DecodeState, ProviderChunk, ProviderKind, RequestOptions,
};
use betterlite_streaming::provider_chunk_decode::dispatch::decode_provider_chunk;
use betterlite_streaming::provider_chunk_decode::handlers::process_chunk;
use betterlite_streaming::stream_chunk_shaping::ChunkShaper;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{GenericChunk, JsonObject, StreamChunk};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::HelperError;

use self::creator::creator_step;
use self::views::{decode_view, error_view};

mod creator;
mod views;

pub(super) fn run(input: &Value) -> Result<Value, Box<HelperError>> {
    let spec = input.get("state").cloned().unwrap_or_default();
    let mut harness = Harness {
        provider: field(&spec, "provider")?,
        state: build_state(&spec)?,
        shaper: build_shaper(&spec)?,
    };
    let steps: Vec<Value> = field(input, "steps")?.unwrap_or_default();
    let mut outputs = Vec::with_capacity(steps.len());
    for step in &steps {
        outputs.push(apply(&mut harness, step)?);
    }
    Ok(Value::Array(outputs))
}

struct Harness {
    provider: Option<String>,
    state: DecodeState,
    shaper: ChunkShaper,
}

const fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000,
        timestamp: 1_700_000_000.5,
    }
}

fn field<T>(value: &Value, key: &str) -> Result<Option<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    let Some(raw) = value.get(key).filter(|raw| !raw.is_null()) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone())
        .map(Some)
        .map_err(|error| failure("InvalidInput", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn build_state(spec: &Value) -> Result<DecodeState, Box<HelperError>> {
    let options = RequestOptions {
        send_stream_usage: field::<bool>(spec, "send_stream_usage")?.unwrap_or(false),
        is_function_call: field::<bool>(spec, "is_function_call")?.unwrap_or(false),
    };
    let kind = if field::<bool>(spec, "custom_provider")?.unwrap_or(false) {
        ProviderKind::Custom
    } else {
        ProviderKind::Builtin
    };
    Ok(DecodeState {
        model: field::<String>(spec, "model")?.unwrap_or_default(),
        kind,
        received_finish_reason: field(spec, "received_finish_reason")?,
        intermittent_finish_reason: field(spec, "intermittent_finish_reason")?,
        sent_first_chunk: field::<bool>(spec, "sent_first_chunk")?.unwrap_or(false),
        complete_response: field::<String>(spec, "complete_response")?.unwrap_or_default(),
        completion_stream: field(spec, "completion_stream")?,
        options,
        repeated_chunk_limit: field::<usize>(spec, "repeated_chunk_limit")?.unwrap_or(100),
    })
}

fn build_shaper(spec: &Value) -> Result<ChunkShaper, Box<HelperError>> {
    let mut shaper = ChunkShaper::new(
        field::<String>(spec, "model")?.unwrap_or_default(),
        field(spec, "provider")?,
        field::<bool>(spec, "merge_reasoning_in_choices")?.unwrap_or(false),
        field::<JsonObject>(spec, "base_hidden_params")?.unwrap_or_default(),
    );
    shaper.response_id = field(spec, "response_id")?;
    shaper.created = field(spec, "created")?;
    shaper.system_fingerprint = field(spec, "system_fingerprint")?;
    shaper.chunks = field::<Vec<StreamChunk>>(spec, "chunks")?.unwrap_or_default();
    shaper.progress.sent_first_chunk = field::<bool>(spec, "sent_first_chunk")?.unwrap_or(false);
    shaper.tool_call = field::<bool>(spec, "tool_call")?.unwrap_or(false);
    shaper.received_finish_reason = field(spec, "received_finish_reason")?;
    shaper.intermittent_finish_reason = field(spec, "intermittent_finish_reason")?;
    shaper.now = fixed_instant;
    shaper.next_id = Uuid::nil;
    Ok(shaper)
}

fn apply(harness: &mut Harness, step: &Value) -> Result<Value, Box<HelperError>> {
    apply_step(harness, step)?;
    let op: String = field(step, "op")?.unwrap_or_default();
    match op.as_str() {
        "decode" => decode_step(harness, step),
        "creator" => creator_step(harness, step),
        "process_chunk" => process_step(harness, step),
        other => Err(failure(
            "InvalidInput",
            format!("unsupported operation {other}"),
        )),
    }
}

fn apply_step(harness: &mut Harness, step: &Value) -> Result<(), Box<HelperError>> {
    if let Some(raw) = step.get("provider") {
        let provider = raw.as_str().map(str::to_owned);
        harness.provider.clone_from(&provider);
        harness.shaper.provider = provider;
    }
    if let Some(model) = field::<String>(step, "model")? {
        harness.state.model.clone_from(&model);
        harness.shaper.model = model;
    }
    if let Some(usage) = field::<bool>(step, "send_stream_usage")? {
        harness.state.options.send_stream_usage = usage;
    }
    if let Some(is_call) = field::<bool>(step, "is_function_call")? {
        harness.state.options.is_function_call = is_call;
    }
    Ok(())
}

fn decode_step(harness: &mut Harness, step: &Value) -> Result<Value, Box<HelperError>> {
    let chunk = chunk_of(step)?;
    match decode_provider_chunk(harness.provider.as_deref(), &chunk, &mut harness.state) {
        Ok(decoded) => Ok(decode_view(&decoded, &harness.state)),
        Err(error) => Ok(error_view(&error)),
    }
}

fn process_step(harness: &mut Harness, step: &Value) -> Result<Value, Box<HelperError>> {
    let text: String = field(step, "chunk")?.unwrap_or_default();
    let processed = process_chunk(&mut harness.state, &text);
    Ok(json!({"text": processed, "complete_response": harness.state.complete_response}))
}

fn chunk_of(step: &Value) -> Result<ProviderChunk, Box<HelperError>> {
    let spec = step.get("chunk").cloned().unwrap_or_default();
    let kind: String = field(&spec, "kind")?.unwrap_or_default();
    match kind.as_str() {
        "text" => {
            let text: String = field(&spec, "value")?.unwrap_or_default();
            Ok(ProviderChunk::Text(text))
        }
        "bytes" => {
            let bytes: Vec<u8> = field(&spec, "value")?.unwrap_or_default();
            Ok(ProviderChunk::Bytes(bytes))
        }
        "generic" => {
            let value = spec.get("value").cloned().unwrap_or(Value::Null);
            let generic: GenericChunk = serde_json::from_value(value)
                .map_err(|error| failure("InvalidInput", error.to_string()))?;
            Ok(ProviderChunk::Generic(Box::new(generic)))
        }
        "json" => {
            let value = spec.get("value").cloned().unwrap_or(Value::Null);
            let parsed = serde_json::from_value(value)
                .map_err(|error| failure("InvalidInput", error.to_string()))?;
            Ok(ProviderChunk::Json(Box::new(parsed)))
        }
        other => Err(failure(
            "InvalidInput",
            format!("unsupported chunk kind {other}"),
        )),
    }
}
