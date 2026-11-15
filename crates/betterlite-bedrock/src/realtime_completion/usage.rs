use std::mem;

use super::error::CompletionError;
use super::records::{
    CompletionEvent, InputTokenDetails, OutputTokenDetails, ResponseDone, ResponseObject,
    ResponseState, ResponseUsage,
};
use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;

// Counts are never clamped, so a cumulative total that shrinks gives a negative delta
pub fn take_usage_delta(session: &mut RealtimeSession) -> Result<ResponseUsage, CompletionError> {
    let latest = session.cumulative_usage.clone();
    let prior = mem::replace(&mut session.reported_usage, latest.clone());
    let diff = |newer: i64, older: i64| {
        newer
            .checked_sub(older)
            .ok_or(CompletionError::UsageOverflow)
    };
    let (latest_total, prior_total) = (&latest.details.total, &prior.details.total);
    Ok(ResponseUsage {
        input_tokens: diff(latest.total_input_tokens, prior.total_input_tokens)?,
        output_tokens: diff(latest.total_output_tokens, prior.total_output_tokens)?,
        total_tokens: diff(latest.total_tokens, prior.total_tokens)?,
        input_token_details: InputTokenDetails {
            audio: diff(
                latest_total.input.speech_tokens,
                prior_total.input.speech_tokens,
            )?,
            text: diff(
                latest_total.input.text_tokens,
                prior_total.input.text_tokens,
            )?,
            cached: 0,
        },
        output_token_details: OutputTokenDetails {
            audio_tokens: diff(
                latest_total.output.speech_tokens,
                prior_total.output.speech_tokens,
            )?,
            text_tokens: diff(
                latest_total.output.text_tokens,
                prior_total.output.text_tokens,
            )?,
        },
    })
}

// Logged only: the client never receives the done event for usage reported after the last turn
pub fn leftover_usage_done_events(
    session: &mut RealtimeSession,
    ids: &dyn IdSource,
) -> Result<Vec<CompletionEvent>, CompletionError> {
    if session.cumulative_usage == session.reported_usage {
        return Ok(Vec::new());
    }
    let usage = take_usage_delta(session)?;
    Ok(vec![CompletionEvent::ResponseDone(ResponseDone {
        event_id: format!("event_{}", ids.next_id()),
        response: ResponseObject {
            object: "realtime.response",
            id: format!("resp_{}", ids.next_id()),
            status: "completed",
            output: None,
            conversation_id: format!("conv_{}", ids.next_id()),
            usage,
        },
    })])
}

// promptEnd, completionEnd and an END_TURN contentEnd all end the response here
pub fn response_done_events(
    session: &mut RealtimeSession,
    state: &mut ResponseState,
    ids: &dyn IdSource,
) -> Result<Vec<CompletionEvent>, CompletionError> {
    let response_id = state.response_id.take();
    state.output_item_id = None;
    state.delta_type = None;
    let (Some(id), Some(conversation_id)) = (response_id, state.conversation_id.clone()) else {
        return Ok(Vec::new());
    };
    let usage = take_usage_delta(session)?;
    Ok(vec![CompletionEvent::ResponseDone(ResponseDone {
        event_id: format!("event_{}", ids.next_id()),
        response: ResponseObject {
            object: "realtime.response",
            id,
            status: "completed",
            output: Some(Vec::new()),
            conversation_id,
            usage,
        },
    })])
}
