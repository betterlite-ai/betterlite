use super::convert::{
    AudioCallWire, ConvertError, DEFAULT_MAX_RETRIES, NoFiles, OPENAI_PROVIDER,
    TranscriptionAnswer, audio_file, bare_string_file, family_for, first_file, max_retries, text,
    timeout, transcription_options,
};
use super::error::{CallFailure, context, missing_key, provider_failure};
use super::session::{AudioSession, SilentAudioLogger};
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::decoder::{
    AudioDecodeLimits, AudioDecoder, NativeAudioDecoder,
};
use betterlite_openai::api::audio_file_input::duration::calculate_request_duration;
use betterlite_openai::api::audio_file_input::{AudioFile, ProcessedAudioFile};
use betterlite_openai::transcription::call::{
    TranscriptionCall, TranscriptionRequest, audio_transcriptions, finish_transcription,
};
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::duration::duration_number;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_openai::transcription::transport::TranscriptionTransport;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_files::CallFile;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::time::Instant;
use uuid::Uuid;

use crate::images::convert::fields::resolve_target;

struct Prepared {
    model: String,
    provider: String,
    processed: ProcessedAudioFile,
    input: String,
    connection: TranscriptionConnection,
    options: IndexMap<String, Json>,
}

pub(crate) async fn run(
    session: &AudioSession,
    call: AudioCallWire,
    files: Vec<(String, CallFile)>,
) -> Result<TranscriptionAnswer, CallFailure> {
    let started = Instant::now();
    let prepared = prepare(session, &call, &files)?;
    let request = TranscriptionRequest {
        model: &prepared.model,
        family: Some(family_for(&prepared.model)),
        optional_params: prepared.options,
        atranscription: call.atranscription,
    };
    let transcription = TranscriptionCall {
        input: prepared.input,
        file: &prepared.processed,
        connection: prepared.connection,
        boundary: Uuid::new_v4(),
    };
    let mut logger = SilentAudioLogger;
    let (outbound, pending) =
        audio_transcriptions(&transcription, &request, &mut logger).map_err(|error| {
            transcription_failure(
                error,
                &prepared.model,
                &prepared.provider,
                &session.redactor,
            )
        })?;
    let sent = TranscriptionTransport::send(&session.client, outbound).await;
    let reply = finish_transcription(pending, sent, &mut logger).map_err(|error| {
        transcription_failure(
            error,
            &prepared.model,
            &prepared.provider,
            &session.redactor,
        )
    })?;
    let response_ms = started.elapsed().as_secs_f64().mul_add(1000.0_f64, 0.0_f64);
    let hidden_params = hidden_params(
        &reply,
        &prepared.processed,
        response_ms,
        &call,
        &prepared.model,
    )
    .map_err(|error| {
        transcription_failure(
            error,
            &prepared.model,
            &prepared.provider,
            &session.redactor,
        )
    })?;
    Ok(TranscriptionAnswer {
        reply: reply.response,
        response_ms,
        hidden_params,
    })
}

fn prepare(
    session: &AudioSession,
    call: &AudioCallWire,
    files: &[(String, CallFile)],
) -> Result<Prepared, CallFailure> {
    bare_string_file(&call.arguments)?;
    let given = text(&call.arguments, "model")?.ok_or_else(|| {
        CallFailure::plain(
            "ValueError",
            String::from(
                "model parameter is required but was None. Please provide a valid model name.",
            ),
            None,
        )
    })?;
    let provider_arg = text(&call.arguments, "custom_llm_provider")?;
    let (model, provider) = resolve_target(&given, provider_arg.as_deref());
    if provider != OPENAI_PROVIDER {
        return Err(unsupported(&model, &provider));
    }
    let chosen = first_file(files).ok_or_else(|| {
        CallFailure::plain(
            "ValueError",
            String::from("file is required for transcription"),
            None,
        )
    })?;
    let (processed, input) =
        audio_file(chosen).map_err(|error| CallFailure::from(ConvertError::Audio(error)))?;
    let connection = transcription_connection(session, call, &model)?;
    Ok(Prepared {
        options: transcription_options(&call.arguments),
        model,
        provider,
        processed,
        input,
        connection,
    })
}

fn transcription_connection(
    session: &AudioSession,
    call: &AudioCallWire,
    model: &str,
) -> Result<TranscriptionConnection, CallFailure> {
    let credentials = session.credentials(&call.settings, &call.arguments)?;
    let api_key = credentials
        .api_key
        .clone()
        .ok_or_else(|| missing_key(model))?;
    let request_timeout = timeout(&call.arguments)?;
    let retries = max_retries(&call.arguments)?.unwrap_or(DEFAULT_MAX_RETRIES);
    Ok(TranscriptionConnection {
        api_key: Some(api_key),
        environment_api_key: None,
        api_base: Some(credentials.api_base),
        timeout: request_timeout,
        max_retries: retries,
        shared_session: None,
    })
}

// The handler's model and headers, main.py's duration and the wrapper's timing.
fn hidden_params(
    reply: &TranscriptionReply,
    processed: &ProcessedAudioFile,
    response_ms: f64,
    call: &AudioCallWire,
    model: &str,
) -> Result<IndexMap<String, Json>, TranscriptionError> {
    let mut hidden = IndexMap::new();
    hidden.insert(
        String::from("model"),
        Json::Text(reply.model.clone().unwrap_or_else(|| model.to_owned())),
    );
    hidden.insert(
        String::from("custom_llm_provider"),
        Json::Text(String::from(reply.custom_llm_provider)),
    );
    let headers = reply
        .provider_headers
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    hidden.insert(String::from("headers"), Json::Object(headers));
    hidden.insert(
        String::from("additional_headers"),
        Json::Object(reply.provider_headers.additional_headers.clone()),
    );
    let duration_missing = !reply.response.extra.contains_key("duration");
    if !call.atranscription || call.file_from_kwargs {
        if duration_missing && let Some(found) = request_duration(&processed.file_content) {
            hidden.insert(
                String::from("audio_transcription_duration"),
                duration_number(Some(found))?,
            );
        }
    } else if let StoredDuration::Known(found) = reply.duration {
        hidden.insert(
            String::from("audio_transcription_duration"),
            duration_number(found)?,
        );
    }
    if let Some(found) = Number::from_f64(response_ms) {
        hidden.insert(String::from("_response_ms"), Json::Number(found));
    }
    Ok(hidden)
}

fn request_duration(content: &[u8]) -> Option<Decimal> {
    let decoder = NativeAudioDecoder {
        limits: AudioDecodeLimits::default(),
    };
    calculate_request_duration(
        &mut AudioFile::Bytes(content),
        &NoFiles,
        AudioDecoder::Available(&decoder),
    )
}

fn transcription_failure(
    error: TranscriptionError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    match &error {
        TranscriptionError::Status(found) => provider_failure(
            error.to_string(),
            Some(found.status),
            model,
            provider,
            redactor,
        ),
        TranscriptionError::Transport { .. } => {
            provider_failure(error.to_string(), None, model, provider, redactor)
        }
        TranscriptionError::MissingApiKey => missing_key(model),
        TranscriptionError::BaseUrl { .. }
        | TranscriptionError::Field { .. }
        | TranscriptionError::Header { .. }
        | TranscriptionError::Logging { .. }
        | TranscriptionError::Utf8 { .. }
        | TranscriptionError::Json { .. }
        | TranscriptionError::NotObject
        | TranscriptionError::NoResponseFields => {
            CallFailure::plain("ValueError", error.to_string(), Some(model.to_owned()))
        }
    }
}

fn unsupported(model: &str, provider: &str) -> CallFailure {
    let message = format!("transcription is not supported for {provider}");
    CallFailure::from_class(&ErrorClass::ApiConnection(Box::new(context(
        message,
        Some(model.to_owned()),
        Some(provider.to_owned()),
    ))))
}
