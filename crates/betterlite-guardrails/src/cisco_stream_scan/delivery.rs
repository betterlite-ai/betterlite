use std::fmt::Display;

use betterlite_streaming::streamed_response_assembly::records::Chunk;

use crate::cisco_conversation::json::Json;
use crate::cisco_conversation::{Fields, InspectMessage};
use crate::cisco_inspection::hooks::{extract_response_messages, record_applied_guardrail};
use crate::cisco_stream_scan::events;
use crate::cisco_stream_scan::rebuild;
use crate::cisco_stream_scan::{StreamItem, api_error_detail};
use crate::cisco_verdict_enforcement::api_error::{ApiErrorDetail, ApiErrorOutcome};
use crate::cisco_verdict_enforcement::block_payload::{BlockPayload, GuardrailException};
use crate::cisco_verdict_enforcement::finalize::FinalizeOutcome;
use crate::cisco_verdict_enforcement::http_exception_to_error_obj;

pub(super) fn stream_outcome(
    outcome: FinalizeOutcome,
    guardrail_name: &str,
) -> Option<Vec<StreamItem>> {
    match outcome {
        FinalizeOutcome::Blocked(payload) => Some(blocked_stream(
            guardrail_name,
            BlockPayload::STATUS_CODE,
            serde_json::to_string(&payload).map_or_else(|_| Json::Null, |text| Json::parse(&text)),
        )),
        FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(detail)) => Some(blocked_stream(
            guardrail_name,
            ApiErrorDetail::STATUS_CODE,
            api_error_detail(&detail),
        )),
        FinalizeOutcome::Allow
        | FinalizeOutcome::Redacted(_)
        | FinalizeOutcome::Monitored
        | FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned) => None,
    }
}

pub(super) fn deliver(
    guardrail_name: &str,
    request_data: &mut Fields,
    chunks: &[Chunk],
    response: &Fields,
    scan_text: &str,
) -> Vec<StreamItem> {
    record_applied_guardrail(request_data, guardrail_name);
    if joined_contents(&extract_response_messages(response)) == scan_text {
        return pass_chunks(chunks);
    }
    rebuild_stream(guardrail_name, response)
}

pub(super) fn pass_chunks(chunks: &[Chunk]) -> Vec<StreamItem> {
    let mut items = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        items.push(StreamItem::Chunk(Box::new(chunk.clone())));
    }
    items
}

pub(super) fn joined_contents(messages: &[InspectMessage]) -> String {
    let mut joined = String::new();
    for (position, message) in messages.iter().enumerate() {
        if position > 0 {
            joined.push(' ');
        }
        joined.push_str(&message.content);
    }
    joined
}

pub(super) fn scan_failure<Failure>(guardrail_name: &str, failure: &Failure) -> Vec<StreamItem>
where
    Failure: Display,
{
    tracing::error!(
        "Cisco AI Defense guardrail ({guardrail_name}): streaming response scan failed: {failure}"
    );
    events::event(guardrail_name, events::SCAN_ERROR)
}

fn blocked_stream(guardrail_name: &str, status_code: u16, detail: Json) -> Vec<StreamItem> {
    tracing::warn!(
        "Cisco AI Defense guardrail ({guardrail_name}): streaming response blocked \u{2014} emitting SSE error event instead of delivering buffered chunks."
    );
    let exception = GuardrailException {
        status_code,
        detail,
    };
    let error_obj = http_exception_to_error_obj(&exception, guardrail_name);
    events::error_object(&error_obj)
}

fn rebuild_stream(guardrail_name: &str, response: &Fields) -> Vec<StreamItem> {
    match rebuild::mock_response_chunks(response) {
        Ok(chunks) => pass_chunks(&chunks),
        Err(error) => scan_failure(guardrail_name, &error),
    }
}
