use std::fmt::Display;
use std::pin::pin;

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_native_stream::retention::RetentionBudget;
use betterlite_streaming::streamed_response_assembly::records::{Chunk, TextField};
use betterlite_streaming::streamed_response_assembly::response::Response;
use futures_util::StreamExt as _;
use futures_util::stream::Stream;

use crate::cisco_configuration::records::Direction;
use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::request::request_messages;
use crate::cisco_conversation::{Fields, InspectMessage, Role};
use crate::cisco_inspection::chat::{ChatInspectionArgs, inspect_chat};
use crate::cisco_inspection::construct::CiscoInspectionGuardrail;
use crate::cisco_inspection::hooks::{PostCallArgs, extract_response_messages};
use crate::cisco_stream_scan::delivery::{
    deliver, joined_contents, pass_chunks, scan_failure, stream_outcome,
};
use crate::cisco_stream_scan::scan_text::chunk_scan_text;
use crate::cisco_verdict_enforcement::api_error::ApiErrorDetail;
use crate::cisco_verdict_enforcement::block_payload::BlockPayload;
use crate::cisco_verdict_enforcement::finalize::ResponseSurface;
use crate::cisco_verdict_enforcement::support::insert_serialized;

mod assembly;
mod delivery;
mod events;
mod rebuild;
mod scan_text;
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    // boxed to stay within 3x of Sse, the limit variant_size_differences enforces
    Chunk(Box<Chunk>),
    Sse(String),
}

pub async fn scan_stream<S, StreamError>(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    chunks: S,
    args: PostCallArgs<'_>,
) -> Result<Vec<StreamItem>, StreamError>
where
    S: Stream<Item = Result<Chunk, StreamError>>,
    StreamError: Display,
{
    let name = guardrail.settings.guardrail_name.as_str();
    if guardrail.settings.inspection_type != InspectionType::Chat || !args.should_run {
        let Buffered::Chunks(buffered) = collect_chunks(chunks, false).await? else {
            return Ok(events::event(name, events::SCAN_ERROR));
        };
        return Ok(pass_chunks(&buffered));
    }
    tracing::debug!("Cisco AI Defense guardrail ({name}): scanning streaming chat response.");
    let Buffered::Chunks(buffered) = collect_chunks(chunks, true).await? else {
        return Ok(events::event(name, events::SCAN_ERROR));
    };
    if buffered.is_empty() {
        return Ok(Vec::new());
    }
    Ok(scan_chat(guardrail, request_data, &buffered, args.key_user_id).await)
}

enum Buffered {
    Chunks(Vec<Chunk>),
    OverBudget,
}

// The buffer holds untrusted provider output, so the API boundary ceilings bound it as they do for noma
async fn collect_chunks<S, StreamError>(
    chunks: S,
    log_failure: bool,
) -> Result<Buffered, StreamError>
where
    S: Stream<Item = Result<Chunk, StreamError>>,
    StreamError: Display,
{
    let limits = ApiLimits::default();
    let mut budget = RetentionBudget {
        max_bytes: limits.max_json_bytes,
        max_chunks: limits.max_items,
        bytes: 0,
        chunks: 0,
    };
    let mut buffered = Vec::new();
    let mut chunks = pin!(chunks);
    while let Some(item) = chunks.next().await {
        match item {
            Ok(chunk) => {
                let size = serde_json::to_string(&chunk).map_or(usize::MAX, |text| text.len());
                if budget.accept(size).is_err() {
                    tracing::error!(
                        "Cisco AI Defense guardrail: streaming response exceeded the buffer limit"
                    );
                    return Ok(Buffered::OverBudget);
                }
                buffered.push(chunk);
            }
            Err(error) => {
                if log_failure {
                    tracing::error!(
                        "Cisco AI Defense guardrail: upstream streaming failed: {error}"
                    );
                }
                return Err(error);
            }
        }
    }
    Ok(Buffered::Chunks(buffered))
}

async fn scan_chat(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    chunks: &[Chunk],
    key_user_id: Option<&str>,
) -> Vec<StreamItem> {
    let name = guardrail.settings.guardrail_name.as_str();
    if first_item_is_text_completion(chunks) {
        tracing::warn!(
            "Cisco AI Defense guardrail ({name}): unsupported streaming chunk shape — failing closed."
        );
        return events::event(name, events::UNSUPPORTED);
    }
    let assembled = match assembly::assemble(chunks).await {
        Ok(Some(assembled)) => assembled,
        Ok(None) => return pass_chunks(chunks),
        Err(error) => return scan_failure(name, &error),
    };
    let Response::Chat(response) = assembled else {
        tracing::warn!(
            "Cisco AI Defense guardrail ({name}): assembled streaming response has unsupported shape — failing closed."
        );
        return events::event(name, events::UNSUPPORTED);
    };
    let scanned_response = match assembly::response_fields(response.as_ref()) {
        Ok(fields) => fields,
        Err(error) => return scan_failure(name, &error),
    };
    inspect_and_deliver(
        guardrail,
        request_data,
        chunks,
        scanned_response,
        key_user_id,
    )
    .await
}

async fn inspect_and_deliver(
    guardrail: &CiscoInspectionGuardrail,
    request_data: &mut Fields,
    chunks: &[Chunk],
    mut response: Fields,
    key_user_id: Option<&str>,
) -> Vec<StreamItem> {
    let name = guardrail.settings.guardrail_name.as_str();
    let (messages, scan_text) = conversation_messages(&response, chunks);
    if messages.is_empty() {
        return pass_chunks(chunks);
    }
    let mut conversation = request_messages(request_data);
    conversation.extend(messages);
    let args = ChatInspectionArgs {
        messages: conversation,
        direction: Direction::Output,
        key_user_id,
    };
    let surface = ResponseSurface::ChatResponse(&mut response);
    let outcome = inspect_chat(guardrail, args, request_data, surface).await;
    if let Some(event) = stream_outcome(outcome, name) {
        return event;
    }
    deliver(name, request_data, chunks, &response, &scan_text)
}

fn conversation_messages(response: &Fields, chunks: &[Chunk]) -> (Vec<InspectMessage>, String) {
    let mut messages = extract_response_messages(response);
    let scan_text = chunk_scan_text(chunks);
    let assembled_text = joined_contents(&messages);
    if !scan_text.is_empty() && !assembled_text.contains(&scan_text) {
        messages.push(InspectMessage {
            role: Role::Assistant,
            content: scan_text.clone(),
        });
    }
    (messages, scan_text)
}

fn first_item_is_text_completion(chunks: &[Chunk]) -> bool {
    chunks.first().is_some_and(|chunk| {
        chunk
            .choices
            .iter()
            .any(|choice| matches!(&choice.text, TextField::Present(_)))
    })
}

pub(crate) fn block_detail(payload: &BlockPayload) -> Json {
    match serde_json::to_string(payload) {
        Ok(text) => Json::parse(&text),
        Err(_) => Json::Null,
    }
}

pub(crate) fn api_error_detail(detail: &ApiErrorDetail) -> Json {
    let mut fields = Fields::new();
    insert_serialized(&mut fields, "error", detail.error);
    insert_serialized(&mut fields, "message", &detail.message);
    insert_serialized(&mut fields, "error_type", &detail.error_type);
    Json::Object(fields)
}
