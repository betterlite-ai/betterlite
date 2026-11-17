use betterlite_core::api::realtime_session_state::logging::SessionInput;
use betterlite_core::api::realtime_session_state::store::SessionMessage;
use betterlite_core::api::realtime_session_state::{
    ConnectionSetup, RealtimeProviderConfig, RealtimeStreaming,
};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::Failure;
use super::failure;
use super::input::{Frame, Input, InputMessage, selection};
use super::snapshot::snapshot;

pub(crate) fn drive(input: &Input) -> HelperResult {
    let mut state = RealtimeStreaming::new(connection(input)?);
    for frame in &input.store {
        state
            .store_message(frame_message(frame))
            .map_err(|error| failure("SessionStateError", error.to_string()))?;
    }
    for message in &input.inputs {
        state.store_input(input_message(message));
    }
    for event in &input.backend_events {
        state.collect_backend_input(event);
        state.detect_transcription_session(event);
    }
    for event in &input.usage_events {
        state.capture_transcription_usage(event);
    }
    if input.flush {
        state
            .flush_unbilled_usage()
            .map_err(|error| failure("SessionStateError", error.to_string()))?;
    }
    Ok(snapshot(&state))
}

fn connection(input: &Input) -> Result<ConnectionSetup, Failure> {
    let provider = input
        .provider
        .as_ref()
        .map(|document| -> Box<dyn RealtimeProviderConfig> { Box::new(document.clone()) });
    Ok(ConnectionSetup {
        client: None,
        backend: None,
        logging: None,
        logging_worker: None,
        provider_config: provider,
        event_normalizer: None,
        logged_real_time_event_types: selection(input.logged_event_types.as_ref())?,
        model: input.model.clone(),
        user_api_key_dict: input.user_api_key_dict.clone(),
        request_data: input.request_data.clone(),
        force_transcription_model: input.force_transcription_model.clone(),
        client_wants_beta: input.client_wants_beta,
        backend_uses_beta_protocol: input.backend_uses_beta_protocol,
    })
}

const fn frame_message(frame: &Frame) -> SessionMessage<'_> {
    match frame {
        Frame::Text { text } => SessionMessage::Text(text.as_str()),
        Frame::Bytes { text } => SessionMessage::Binary(text.as_bytes()),
        Frame::Record { values } => SessionMessage::Record(values),
    }
}

const fn input_message(message: &InputMessage) -> SessionInput<'_> {
    match message {
        InputMessage::Text { text } => SessionInput::Text(text.as_str()),
        InputMessage::Record { values } => SessionInput::Record(values),
    }
}
