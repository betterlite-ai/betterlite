pub mod client;
pub mod logging;
pub mod reply;
pub mod request;

use std::time::Duration;

use betterlite_http::client_kind::ClientKind as CallKind;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::InvalidUrl;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::transcription::decode::{ReplyFormat, reply_format};
use betterlite_openai::transcription::error::TranscriptionError as ReplyError;
use betterlite_openai::transcription::request::{TranscriptionData, logged_data};
use http::header::InvalidHeaderValue;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;
use uuid::Uuid;

use self::client::{ClientWiring, Connection, SelectedClient, request_auth, select_client};
use self::logging::{AzureTranscriptionLogger, AzureTranscriptionPreCall};
use self::reply::log_failure;
use self::request::{OutboundTarget, outbound_request};
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind, SuppliedClient};

#[derive(Debug, Error)]
pub enum AzureTranscriptionError {
    #[error("azure_client is not an instance of AzureOpenAI")]
    NotAzureClient,
    #[error("async_azure_client is not an instance of AsyncAzureOpenAI")]
    NotAsyncAzureClient,
    #[error("Unable to handle auth")]
    UnableToHandleAuth,
    #[error(
        "Expected `azure_ad_token_provider` argument to return a string but it returned {returned}"
    )]
    TokenNotString { returned: String },
    #[error("{message}")]
    TokenProvider { class: String, message: String },
    #[error(transparent)]
    Client(#[from] EndpointError),
    #[error(
        "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable"
    )]
    MissingBase,
    #[error("the transcription client base URL is not a URL")]
    Url {
        #[source]
        source: InvalidUrl,
    },
    #[error("the transcription client URL is not absolute")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the transcription field `{field}` {problem}")]
    Field {
        field: String,
        problem: &'static str,
    },
    #[error("the transcription API key is not a header value")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the transcription request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the transcription logging hook failed")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error(transparent)]
    Reply(#[from] ReplyError),
}

impl AzureTranscriptionError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::NotAzureClient | Self::NotAsyncAzureClient => Some(500),
            Self::Client(error) => error.status(),
            Self::Status(error) => Some(error.status),
            Self::UnableToHandleAuth
            | Self::TokenNotString { .. }
            | Self::TokenProvider { .. }
            | Self::MissingBase
            | Self::Url { .. }
            | Self::BaseUrl { .. }
            | Self::Field { .. }
            | Self::Header { .. }
            | Self::Transport { .. }
            | Self::Logging { .. }
            | Self::Reply(_) => None,
        }
    }
}

// The arguments AzureAudioTranscription.audio_transcriptions takes, as one call.
pub struct AzureTranscriptionCall<'data> {
    // get_audio_file_name of the audio the file was processed from
    pub input: String,
    pub file: &'data ProcessedAudioFile,
    // the caller's stable id: the pre_call input names the audio with it, and the multipart boundary uses it
    pub boundary: Uuid,
    pub model: &'data str,
    pub optional_params: IndexMap<String, Json>,
    pub atranscription: bool,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    // the owned calls take these arguments but hand only litellm_params to the client builder
    pub azure_ad_token: Option<SecretString>,
    pub max_retries: Option<i64>,
    pub timeout: Duration,
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub session: SessionKind,
    pub refresh_enabled: bool,
}

// What the reply handling needs from the part of the call that ran before the send.
pub struct Pending {
    body: IndexMap<String, Json>,
    input: String,
    model: Option<String>,
    kind: CallKind,
    format: ReplyFormat,
    authorization: String,
    api_base: String,
}

// AzureAudioTranscription.audio_transcriptions: build {"model", "file", **options}, then call the sync or async client.
pub fn audio_transcriptions(
    call: &AzureTranscriptionCall<'_>,
    wiring: &mut ClientWiring<'_, '_>,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<(OutboundRequest, Pending), AzureTranscriptionError> {
    let data = transcription_data(call.model, &call.optional_params);
    if call.atranscription {
        return async_audio_transcriptions(call, &data, wiring, logger);
    }
    begin(call, &data, CallKind::Sync, wiring, logger)
}

// AzureAudioTranscription.async_audio_transcriptions: the same exchange on the async client, whose failure is logged before it is rethrown.
pub fn async_audio_transcriptions(
    call: &AzureTranscriptionCall<'_>,
    data: &TranscriptionData,
    wiring: &mut ClientWiring<'_, '_>,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<(OutboundRequest, Pending), AzureTranscriptionError> {
    begin(call, data, CallKind::Asynchronous, wiring, logger)
        .inspect_err(|error| log_failure(&call.input, error, logger))
}

fn transcription_data(model: &str, optional_params: &IndexMap<String, Json>) -> TranscriptionData {
    let mut options = optional_params.clone();
    let requested = options
        .shift_remove("model")
        .unwrap_or_else(|| Json::Text(model.to_owned()));
    TranscriptionData {
        model: Some(requested),
        options,
    }
}

// LiteLLM logs the call before the client builds its request, and the same data rides on the reply log
fn log_pre_call(
    call: &AzureTranscriptionCall<'_>,
    data: &TranscriptionData,
    selected: &SelectedClient<'_>,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<IndexMap<String, Json>, AzureTranscriptionError> {
    let body = logged_data(data, call.file);
    logger
        .pre_call(AzureTranscriptionPreCall {
            input: &format!("audio_file_{}", call.boundary),
            authorization: &selected.authorization,
            api_base: &selected.base_url,
            data: body.clone(),
        })
        .map_err(|source| AzureTranscriptionError::Logging { source })?;
    Ok(body)
}

fn begin(
    call: &AzureTranscriptionCall<'_>,
    data: &TranscriptionData,
    kind: CallKind,
    wiring: &mut ClientWiring<'_, '_>,
    logger: &mut dyn AzureTranscriptionLogger,
) -> Result<(OutboundRequest, Pending), AzureTranscriptionError> {
    let selected = select_client(&connection(call), kind, wiring)?;
    let body = log_pre_call(call, data, &selected, logger)?;
    // the SDK decides the auth header after the call is logged, so a failure is logged like any other
    let auth = request_auth(
        selected.ad_token,
        selected.provider.as_ref(),
        selected.api_key,
    )?;
    let outbound = outbound_request(
        &OutboundTarget {
            base_url: selected.base_url.as_str(),
            api_version: selected.api_version.as_deref(),
            auth: &auth,
            deadline: call.timeout,
        },
        data,
        call.file,
        call.boundary.to_string(),
    )?;
    let pending = Pending {
        body,
        input: call.input.clone(),
        model: Some(call.model.to_owned()),
        kind,
        format: reply_format(&data.options)?,
        authorization: selected.authorization,
        api_base: selected.base_url,
    };
    Ok((outbound, pending))
}

fn connection(call: &AzureTranscriptionCall<'_>) -> Connection {
    Connection {
        model: call.model.to_owned(),
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        api_version: call.api_version.clone(),
        params: call.params.clone(),
        provider: call.provider.clone(),
        supplied: call.supplied.clone(),
        session: call.session,
        refresh_enabled: call.refresh_enabled,
    }
}
