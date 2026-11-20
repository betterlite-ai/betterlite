pub mod config;
pub mod error;
pub mod reply;
pub mod wire;

use std::str;
use std::time::{Duration, Instant};

use betterlite_core::api::vector_store_http_write::error::handle_error;
use betterlite_core::api::vector_store_http_write::service::{
    ClientKind, LogContext, SuppliedClient,
};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::client::SharedSession;
use indexmap::IndexMap;
use url::Url;

use crate::api::transcription_contract::AudioFile;

use self::config::{ClientSource, Headers, TranscriptionHttpConfig, UrlInput, Validation};
use self::error::{TranscriptionHttpError, send_failure_facts, status_facts};
use self::reply::TranscriptionReply;

// The arguments llm_http_handler.py's audio_transcriptions hands its two helpers
pub struct TranscriptionCall<'call> {
    pub model: &'call str,
    pub audio_file: AudioFile<'call>,
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
    pub api_key: Option<&'call str>,
    pub api_base: Option<&'call str>,
    pub custom_llm_provider: &'call str,
    pub headers: Option<&'call Headers>,
    pub atranscription: Json,
    pub timeout: Option<Duration>,
    // accepted and never read: these handlers build no retry loop
    pub max_retries: u32,
    pub shared_session: Option<&'call SharedSession>,
}

pub struct TranscriptionDeps<'deps, C: TranscriptionHttpConfig, S: ClientSource> {
    pub config: Option<&'deps C>,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub(super) struct Prepared {
    pub(super) url: Url,
    pub(super) headers: Headers,
    pub(super) data: AudioTranscriptionRequestData,
}

pub async fn audio_transcriptions<C, S>(
    deps: &TranscriptionDeps<'_, C, S>,
    call: &TranscriptionCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<TranscriptionReply, TranscriptionHttpError>
where
    C: TranscriptionHttpConfig,
    S: ClientSource,
{
    let config = require_config(deps.config, call.model, call.custom_llm_provider)?;
    // `atranscription is True`, so only a boolean selects the async client and an integer does not
    let asynchronous = matches!(call.atranscription, Json::Bool(true));
    let mode = if asynchronous {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    exchange(deps, config, call, mode, log).await
}

pub async fn async_audio_transcriptions<'deps, C, S>(
    deps: &'deps TranscriptionDeps<'deps, C, S>,
    call: &TranscriptionCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<TranscriptionReply, TranscriptionHttpError>
where
    C: TranscriptionHttpConfig,
    S: ClientSource,
{
    exchange(
        deps,
        require_config(deps.config, call.model, call.custom_llm_provider)?,
        call,
        ClientKind::Asynchronous,
        log,
    )
    .await
}

// _prepare_audio_transcription_request, then both posts, then _transform_audio_transcription_response
async fn exchange<C, S>(
    deps: &TranscriptionDeps<'_, C, S>,
    config: &C,
    call: &TranscriptionCall<'_>,
    mode: ClientKind,
    log: &mut LogContext<'_, '_>,
) -> Result<TranscriptionReply, TranscriptionHttpError>
where
    C: TranscriptionHttpConfig,
    S: ClientSource,
{
    let prepared = prepare(config, call, log)?;
    let request = wire::wire_request(prepared, call.timeout)?;
    let client = wire::select_client(deps, mode, call.litellm_params, call.shared_session)?;
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let answer = reply.map_err(|error| {
        TranscriptionHttpError::Provider(handle_error(
            &send_failure_facts(&error, mode, call.timeout, started.elapsed()),
            None,
        ))
    })?;
    if (200..300).contains(&answer.status) {
        return reply::transform_response(config, call.optional_params, &answer);
    }
    Err(TranscriptionHttpError::Provider(handle_error(
        &status_facts(&answer),
        None,
    )))
}

fn require_config<'deps, C>(
    config: Option<&'deps C>,
    model: &str,
    provider: &str,
) -> Result<&'deps C, TranscriptionHttpError> {
    config.ok_or_else(|| TranscriptionHttpError::MissingProviderConfig {
        model: model.to_owned(),
        provider: provider.to_owned(),
    })
}

// validate_environment, get_complete_url, the request transform, the content_type override and the pre-call
fn prepare<C>(
    config: &C,
    call: &TranscriptionCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Prepared, TranscriptionHttpError>
where
    C: TranscriptionHttpConfig,
{
    let empty = Headers::new();
    let messages: [Object; 0] = [];
    let mut headers = config.validate_environment(&Validation {
        api_key: call.api_key,
        headers: call.headers.unwrap_or(&empty),
        model: call.model,
        messages: &messages,
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
    })?;
    let url_text = config.complete_transcription_url(&UrlInput {
        api_base: call.api_base,
        api_key: call.api_key,
        model: call.model,
        optional_params: call.optional_params,
        litellm_params: call.litellm_params,
    })?;
    let data = config.transform_audio_request(
        call.model,
        &call.audio_file,
        call.optional_params,
        call.litellm_params,
    )?;
    // A content_type of "" still overrides the validated headers
    if let Some(content_type) = &data.content_type {
        headers.insert(String::from("Content-Type"), content_type.clone());
    }
    log_pre_call(&url_text, &headers, &data.data, call.optional_params, log)?;
    let url = Url::parse(&url_text).map_err(|source| TranscriptionHttpError::Url {
        url: url_text,
        source,
    })?;
    Ok(Prepared { url, headers, data })
}

fn log_pre_call(
    url: &str,
    headers: &Headers,
    data: &AudioTranscriptionData,
    optional_params: &Object,
    log: &mut LogContext<'_, '_>,
) -> Result<(), TranscriptionHttpError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.clone()));
    }
    let input = match optional_params.get("query") {
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(TranscriptionHttpError::Encode)?
        }
        None => Json::Text(String::new()),
    };
    let record = PreCall {
        input,
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.to_owned()),
            headers: Some(logged),
            complete_input_dict: complete_input(data).map_err(TranscriptionHttpError::Encode)?,
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)?;
    Ok(())
}

// `data or {}`: an empty object or empty bytes logs {}, other bytes log as text when they are UTF-8
fn complete_input(data: &AudioTranscriptionData) -> Result<Option<RawBody>, serde_json::Error> {
    match data {
        AudioTranscriptionData::Object(fields) => Ok(Some(RawBody::Object(logged_object(fields)?))),
        AudioTranscriptionData::Bytes(bytes) if bytes.is_empty() => {
            Ok(Some(RawBody::Object(IndexMap::new())))
        }
        AudioTranscriptionData::Bytes(bytes) => Ok(str::from_utf8(bytes)
            .ok()
            .map(|text| RawBody::Text(text.to_owned()))),
    }
}

fn logged_object(fields: &Object) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut logged = IndexMap::new();
    for (name, value) in fields {
        let converted = serde_json::from_value(value.clone())?;
        logged.insert(name.clone(), converted);
    }
    Ok(logged)
}

mod tests;
