use betterlite_error::error::details::{
    MidStreamFallbackDetails, MidStreamStatus, TimeoutDetails, TimeoutStatus,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use super::errors::DriverError;
use super::live::Event;
use super::source::SourceFailure;
use crate::api::responses_stream_events::ResponsesStreamBaseState;
use crate::api::responses_stream_primitives::duration::StreamDurationTimeout;

// LiteLLM's PRE_OUTPUT_LIFECYCLE_EVENT_TYPES, the three events before a response body starts.
const PRE_OUTPUT_EVENT_TYPES: [&str; 3] = [
    "response.created",
    "response.in_progress",
    "response.queued",
];

pub(super) fn source_error(failure: SourceFailure) -> DriverError {
    match failure {
        SourceFailure::Transport(failure) => DriverError::Transport(failure),
        SourceFailure::Decode(message) => DriverError::Class {
            class: unclassified(message),
        },
    }
}

pub(super) fn unclassified(message: String) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message,
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

pub(super) fn timeout_class(timeout: &StreamDurationTimeout) -> ErrorClass {
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: ErrorContext {
            message: timeout.message.clone(),
            model: Some(timeout.model.clone()),
            provider: Some(timeout.custom_llm_provider.clone()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        original_status: TimeoutStatus(timeout.status_code),
    }))
}

pub(super) fn context(message: String, state: &ResponsesStreamBaseState) -> ErrorContext {
    ErrorContext {
        message,
        model: Some(state.model.clone()),
        provider: Some(state.custom_llm_provider.clone().unwrap_or_default()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

pub(super) fn stream_ended_early_message(state: &ResponsesStreamBaseState) -> String {
    let provider = state
        .custom_llm_provider
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or("provider");
    format!(
        "{provider} closed the responses stream before any terminal event (response.completed, response.incomplete or response.failed)"
    )
}

pub(super) fn fallback_error(
    state: &ResponsesStreamBaseState,
    original: &str,
    status: u16,
) -> ErrorClass {
    ErrorClass::MidStreamFallback(Box::new(MidStreamFallbackDetails {
        context: context(format!("litellm.MidStreamFallbackError: {original}"), state),
        original_status: MidStreamStatus(status),
        generated_content: String::new(),
        is_pre_first_chunk: !state.yielded_first_chunk,
    }))
}

pub(super) fn note_yielded_event(state: &mut ResponsesStreamBaseState, event: &Event) {
    state.yielded_first_chunk = true;
    if !is_lifecycle(event) {
        state.output_started = true;
    }
}

fn is_lifecycle(event: &Event) -> bool {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => matches!(
            known.as_ref(),
            KnownResponseEvent::ResponseCreated(_) | KnownResponseEvent::ResponseInProgress(_)
        ),
        ResponsesAPIStreamingResponse::Generic(record) => {
            PRE_OUTPUT_EVENT_TYPES.contains(&record.r#type.as_str())
        }
    }
}
