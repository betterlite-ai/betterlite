use betterlite_http::client_kind::ClientKind as CallKind;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::transcription::decode::{DecodedReply, decode_transcription_reply};
use betterlite_openai::transcription::duration::{duration_number, srt_vtt_duration};
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};

use super::logging::{AzureTranscriptionLogger, AzureTranscriptionPostCall};
use super::{AzureTranscriptionError, Pending};

pub(super) fn log_failure(
    input: &str,
    error: &AzureTranscriptionError,
    logger: &mut dyn AzureTranscriptionLogger,
) {
    logger.post_call(AzureTranscriptionPostCall {
        input,
        authorization: None,
        api_base: None,
        original_response: Json::Text(error.to_string()),
        data: None,
    });
}

// The reply half of the call; only the async path logs a failure.
pub fn finish_transcription(
    pending: Pending,
    sent: Result<UpstreamResponse, TransportError>,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<TranscriptionReply, AzureTranscriptionError> {
    let kind = pending.kind;
    let input = pending.input.clone();
    let outcome = sent
        .map_err(|source| AzureTranscriptionError::Transport { source })
        .and_then(|response| reply(&response, pending, logger));
    if let (CallKind::Asynchronous, Err(error)) = (kind, &outcome) {
        log_failure(&input, error, logger);
    }
    outcome
}

fn reply(
    response: &UpstreamResponse,
    pending: Pending,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<TranscriptionReply, AzureTranscriptionError> {
    if !(200..300).contains(&response.status) {
        return Err(AzureTranscriptionError::Status(Box::new(
            StatusError::from_response(response.status, &response.body),
        )));
    }
    let headers = flatten_headers(&response.headers);
    let content_type = headers.get("content-type").map(String::as_str);
    let decoded = decode_transcription_reply(&response.body, content_type, pending.format)?;
    let asynchronous = pending.kind == CallKind::Asynchronous;
    let duration = measured_duration(pending.kind, &decoded);
    let original_response = with_duration(decoded.original_response, duration)?;
    logger.post_call(AzureTranscriptionPostCall {
        input: &pending.input,
        authorization: asynchronous.then_some(pending.authorization.as_str()),
        api_base: asynchronous.then_some(pending.api_base.as_str()),
        original_response: original_response.clone(),
        data: Some(pending.body),
    });
    Ok(TranscriptionReply {
        response: decoded.response,
        original_response,
        // convert_to_model_response_object only stores the headers on the async branch
        provider_headers: if asynchronous {
            set_provider_response_headers(&AdditionalHeaders::default(), &headers)
        } else {
            ProviderHeaderProjection::default()
        },
        model: pending.model,
        custom_llm_provider: "azure",
        duration,
    })
}

// only the async path reads a duration, and only off a reply the SDK parsed as plain text
fn measured_duration(kind: CallKind, decoded: &DecodedReply) -> StoredDuration {
    match (kind, decoded.textual) {
        (CallKind::Asynchronous, true) => {
            StoredDuration::Known(decoded.response.text.as_deref().and_then(srt_vtt_duration))
        }
        (CallKind::Sync, _) | (CallKind::Asynchronous, false) => StoredDuration::Absent,
    }
}

// convert_to_model_response_object puts the duration under a private key before the reply is logged
fn with_duration(
    mut original_response: Json,
    duration: StoredDuration,
) -> Result<Json, AzureTranscriptionError> {
    if let (StoredDuration::Known(found), Json::Object(fields)) = (duration, &mut original_response)
    {
        fields.insert(
            String::from("_audio_transcription_duration"),
            duration_number(found)?,
        );
    }
    Ok(original_response)
}
