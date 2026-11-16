use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::tool_permission::ToolPermissionGuardrail;
use betterlite_guardrails::tool_permission::types::object_mapping;
use betterlite_guardrails::tool_permission_responses::buffer_overflow::{
    DEFAULT_MAX_STREAM_CHUNKS, StreamBufferLimits,
};
use betterlite_guardrails::tool_permission_responses::error::ResponseToolError;
use betterlite_guardrails::tool_permission_responses::stream_services::StreamServices;
use betterlite_guardrails::tool_permission_responses::streaming::streaming_iterator_hook;
use serde_json::Value;
use tokio::sync::mpsc;

use betterlite_oracle_helper_api::helper::HelperResult;

use super::support::{build_guardrail, failure, runtime, services};
use super::views::{blocked_view, choices_view};
use super::{DEFAULT_NOW, Input};

fn stream_items(input: &Input) -> Vec<StreamItem> {
    if input.raw_chunks.is_empty() {
        input
            .chunks
            .iter()
            .map(|chunk| StreamItem::Json(GuardrailValue::Object(chunk.clone())))
            .collect()
    } else {
        input
            .raw_chunks
            .iter()
            .map(|chunk| StreamItem::Bytes(chunk.clone().into_bytes()))
            .collect()
    }
}

fn raw_chunk_view(bytes: &[u8]) -> Value {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return serde_json::json!({ "events": Value::Array(Vec::new()) });
    };
    let events: Vec<Value> = text.split("\n\n").filter_map(event_object).collect();
    serde_json::json!({ "events": Value::Array(events) })
}

fn event_object(frame: &str) -> Option<Value> {
    let payload = frame
        .lines()
        .find_map(|line| line.trim().strip_prefix("data:"))?
        .trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    serde_json::from_str(payload).ok()
}

fn stream_view(items: &[StreamItem]) -> Value {
    let projected: Vec<Value> = items
        .iter()
        .map(|item| match item {
            StreamItem::Json(value) => {
                let Some(fields) = object_mapping(value) else {
                    return Value::Null;
                };
                serde_json::json!({ "choices": choices_view(fields) })
            }
            StreamItem::Bytes(bytes) => raw_chunk_view(bytes),
        })
        .collect();
    Value::Array(projected)
}

async fn run_stream_hook(
    guardrail: &ToolPermissionGuardrail,
    limits: StreamBufferLimits,
    services: &StreamServices<'_>,
    items: Vec<StreamItem>,
) -> (Result<(), ResponseToolError>, Vec<StreamItem>) {
    let capacity = items.len().max(1);
    let (sender, receiver) = mpsc::channel(capacity);
    let (output, mut output_receiver) = mpsc::channel(64);
    let feed = async move {
        for item in items {
            if sender.send(item).await.is_err() {
                break;
            }
        }
    };
    let hook = streaming_iterator_hook(guardrail, limits, services, receiver, output);
    let collect = async {
        let mut chunks = Vec::new();
        while let Some(item) = output_receiver.recv().await {
            chunks.push(item);
        }
        chunks
    };
    let ((), result, collected) = tokio::join!(feed, hook, collect);
    (result, collected)
}

pub(crate) fn streaming(input: &Input) -> HelperResult {
    let guardrail = build_guardrail(input)?;
    let services = services(DEFAULT_NOW)?;
    let limits = StreamBufferLimits {
        max_chunks: input.max_chunks.unwrap_or(DEFAULT_MAX_STREAM_CHUNKS),
        max_bytes: ApiLimits::default().max_json_bytes,
    };
    let runtime = runtime()?;
    let (result, collected) = runtime.block_on(run_stream_hook(
        &guardrail,
        limits,
        &services,
        stream_items(input),
    ));
    let blocked = match result {
        Ok(()) => Value::Null,
        Err(ResponseToolError::Guardrail(error)) => match blocked_view(&error) {
            Some(blocked) => blocked,
            None => return Err(failure("GuardrailFailure", error.to_string())),
        },
        Err(other) => return Err(failure("ResponseToolError", other.to_string())),
    };
    Ok(serde_json::json!({
        "chunks": stream_view(&collected),
        "blocked": blocked,
    }))
}
