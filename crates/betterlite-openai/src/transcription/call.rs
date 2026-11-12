use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::call_helpers::headers::{AdditionalHeaders, set_provider_response_headers};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use uuid::Uuid;

use super::client::{ClientKind, TranscriptionConnection, select_client};
use super::config::TranscriptionFamily;
use super::decode::{ReplyFormat, decode_transcription_reply, reply_format};
use super::duration::{duration_number, srt_vtt_duration};
use super::error::TranscriptionError;
use super::logging::{TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall};
use super::reply::{StoredDuration, TranscriptionReply};
use super::request::{TranscriptionData, logged_data, outbound_request, prepare_data};
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::files::error::StatusError;
use crate::files::stream::flatten_headers;

pub struct TranscriptionCall<'file> {
    // get_audio_file_name of the audio the file was processed from
    pub input: String,
    pub file: &'file ProcessedAudioFile,
    pub connection: TranscriptionConnection,
    pub boundary: Uuid,
}

pub struct TranscriptionRequest<'model> {
    pub model: &'model str,
    pub family: Option<TranscriptionFamily>,
    pub optional_params: IndexMap<String, Json>,
    pub atranscription: bool,
}

// What the reply handling needs from the part of the call that ran before the send.
pub struct Pending {
    body: IndexMap<String, Json>,
    input: String,
    model: Option<String>,
    kind: ClientKind,
    format: ReplyFormat,
}

// OpenAIAudioTranscription.audio_transcriptions: build the data, then the sync client or the async one.
pub fn audio_transcriptions(
    call: &TranscriptionCall<'_>,
    request: &TranscriptionRequest<'_>,
    logger: &mut dyn TranscriptionLogger,
) -> Result<(OutboundRequest, Pending), TranscriptionError> {
    let data = prepare_data(request.family, request.model, &request.optional_params)?;
    if request.atranscription {
        return async_audio_transcriptions(call, &data, logger);
    }
    let model = Some(request.model.to_owned());
    begin(call, &data, (ClientKind::Sync, model), logger)
}

// async_audio_transcriptions: the same exchange on the async client; any failure is logged before it is rethrown.
pub fn async_audio_transcriptions(
    call: &TranscriptionCall<'_>,
    data: &TranscriptionData,
    logger: &mut dyn TranscriptionLogger,
) -> Result<(OutboundRequest, Pending), TranscriptionError> {
    let model = match &data.model {
        Some(Json::Text(name)) => Some(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
        None => Some(String::from("whisper-1")),
    };
    let started = begin(call, data, (ClientKind::Async, model), logger);
    if let Err(error) = &started {
        log_failure(&call.input, error, logger);
    }
    started
}

fn begin(
    call: &TranscriptionCall<'_>,
    data: &TranscriptionData,
    (kind, model): (ClientKind, Option<String>),
    logger: &mut dyn TranscriptionLogger,
) -> Result<(OutboundRequest, Pending), TranscriptionError> {
    let client = select_client(&call.connection, kind)?;
    let body = logged_data(data, call.file);
    logger
        .pre_call(TranscriptionPreCall {
            api_base: client.base_url.as_str(),
            data: body.clone(),
        })
        .map_err(|source| TranscriptionError::Logging { source })?;
    let outbound = outbound_request(&client, data, call.file, call.boundary.to_string())?;
    let pending = Pending {
        body,
        input: call.input.clone(),
        model,
        kind,
        format: reply_format(&data.options)?,
    };
    Ok((outbound, pending))
}

fn log_failure(input: &str, error: &TranscriptionError, logger: &mut dyn TranscriptionLogger) {
    logger.post_call(TranscriptionPostCall {
        input,
        original_response: Json::Text(error.to_string()),
        data: None,
    });
}

// The reply half of the call; only the async path logs a failure.
pub fn finish_transcription(
    pending: Pending,
    sent: Result<UpstreamResponse, TransportError>,
    logger: &mut dyn TranscriptionLogger,
) -> Result<TranscriptionReply, TranscriptionError> {
    let kind = pending.kind;
    let input = pending.input.clone();
    let outcome = sent
        .map_err(|source| TranscriptionError::Transport { source })
        .and_then(|response| reply(&response, pending, logger));
    if let (ClientKind::Async, Err(error)) = (kind, &outcome) {
        log_failure(&input, error, logger);
    }
    outcome
}

fn reply(
    response: &UpstreamResponse,
    pending: Pending,
    logger: &mut dyn TranscriptionLogger,
) -> Result<TranscriptionReply, TranscriptionError> {
    if !(200..300).contains(&response.status) {
        return Err(TranscriptionError::Status(Box::new(
            StatusError::from_response(response.status, &response.body),
        )));
    }
    let headers = flatten_headers(&response.headers);
    let content_type = headers.get("content-type").map(String::as_str);
    let decoded = decode_transcription_reply(&response.body, content_type, pending.format)?;
    logger.response_headers(&headers);
    let duration = stored_duration(
        pending.kind,
        pending.format,
        decoded.response.text.as_deref(),
    );
    let mut original_response = decoded.original_response;
    if let (StoredDuration::Known(found), Json::Object(fields)) = (duration, &mut original_response)
    {
        fields.insert(
            String::from("_audio_transcription_duration"),
            duration_number(found)?,
        );
    }
    logger.post_call(TranscriptionPostCall {
        input: &pending.input,
        original_response: original_response.clone(),
        data: Some(pending.body),
    });
    Ok(TranscriptionReply {
        response: decoded.response,
        original_response,
        provider_headers: set_provider_response_headers(&AdditionalHeaders::default(), &headers),
        model: pending.model,
        custom_llm_provider: "openai",
        duration,
    })
}

// only the async path reads a duration, and only off a textual reply
fn stored_duration(kind: ClientKind, format: ReplyFormat, text: Option<&str>) -> StoredDuration {
    match (kind, format) {
        (ClientKind::Async, ReplyFormat::Text) => {
            StoredDuration::Known(text.and_then(srt_vtt_duration))
        }
        (ClientKind::Sync, ReplyFormat::Json | ReplyFormat::Text)
        | (ClientKind::Async, ReplyFormat::Json) => StoredDuration::Absent,
    }
}
