use betterlite_core::api::transcription_call::dispatch::{
    TranscriptionAnswer, TranscriptionProviderReply,
};
use betterlite_core::api::transcription_call::target::OpenAiTranscription;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::transcription::call::{
    TranscriptionCall, TranscriptionRequest, audio_transcriptions, finish_transcription,
};
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::logging::{
    TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall,
};
use betterlite_openai::transcription::transport::TranscriptionTransport;
use indexmap::IndexMap;

use super::DispatchFailure;

// B515's audio_transcriptions, then the reply half over the mock
pub(super) async fn transcribe(
    operation: OpenAiTranscription<'_>,
    client: &ProxiedClient,
) -> Result<TranscriptionAnswer, DispatchFailure> {
    let input = operation.input;
    let call = TranscriptionCall {
        input: input.input.to_owned(),
        file: input.file,
        connection: TranscriptionConnection {
            api_key: input.api_key.cloned(),
            environment_api_key: None,
            api_base: input.api_base.map(str::to_owned),
            timeout: input.timeout,
            max_retries: input.max_retries,
            shared_session: input.shared_session.cloned(),
        },
        boundary: input.boundary,
    };
    let request = TranscriptionRequest {
        model: input.model,
        family: operation.family,
        optional_params: input.optional_params_map(),
        atranscription: input.atranscription,
    };
    let mut logger = Silent;
    let (outbound, pending) =
        audio_transcriptions(&call, &request, &mut logger).map_err(publish)?;
    // The pinned OpenAI SDK reads no openai.organization global, so no organization header is sent
    let sent = TranscriptionTransport::send(client, outbound).await;
    let reply = finish_transcription(pending, sent, &mut logger).map_err(publish)?;
    Ok(TranscriptionAnswer::Reply(Box::new(
        TranscriptionProviderReply {
            response: reply.response,
            provider_headers: reply.provider_headers,
        },
    )))
}

fn publish(error: TranscriptionError) -> DispatchFailure {
    match error {
        TranscriptionError::Status(status) => DispatchFailure::Provider {
            status: Some(status.status),
            message: status.message,
        },
        other @ (TranscriptionError::MissingApiKey
        | TranscriptionError::BaseUrl { .. }
        | TranscriptionError::Field { .. }
        | TranscriptionError::Header { .. }
        | TranscriptionError::Transport { .. }
        | TranscriptionError::Logging { .. }
        | TranscriptionError::Utf8 { .. }
        | TranscriptionError::Json { .. }
        | TranscriptionError::NotObject
        | TranscriptionError::NoResponseFields) => DispatchFailure::Unsupported {
            message: other.to_string(),
        },
    }
}

// The oracle compares the exchanged record, not the logging hooks
struct Silent;

impl TranscriptionLogger for Silent {
    fn pre_call(&mut self, _record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        Ok(())
    }

    fn response_headers(&mut self, _headers: &IndexMap<String, String>) {}

    fn post_call(&mut self, _record: TranscriptionPostCall<'_>) {}
}
