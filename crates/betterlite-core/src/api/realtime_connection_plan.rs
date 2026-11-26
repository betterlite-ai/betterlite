pub mod azure;
pub mod metadata;
pub mod parameters;
mod tests;
pub mod vertex;

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::tls::Verification;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::url::{QueryParams, construct_url};
use secrecy::SecretString;
use thiserror::Error;

use crate::api::realtime_backend_transport::dial::{RealtimeSsl, realtime_ssl};
use crate::api::realtime_connect::token::VertexTokenError;
use crate::api::realtime_connect::{
    AZURE_DEFAULT_API_VERSION, QUERY_MODEL_KEY, RealtimeConnectError,
};

use self::azure::{AzureAdTokens, azure_connection, azure_protocol, transcription_query};
use self::metadata::RealtimeModelMetadata;
use self::parameters::{
    ResolvedProbeParameters, bearer_headers, merge_stored_credentials, resolve_probe_parameters,
};
use self::vertex::VertexProbeConfig;

pub const REALTIME_PROTOCOL_KEY: &str = "realtime_protocol";
pub const AZURE_PROVIDER: &str = "azure";
pub const OPENAI_PROVIDER: &str = "openai";
pub const XAI_PROVIDER: &str = "xai";
pub const VERTEX_AI_PROVIDER: &str = "vertex_ai";
pub const OPENAI_DEFAULT_API_BASE: &str = "https://api.openai.com/";
pub const XAI_DEFAULT_API_BASE: &str = "https://api.x.ai/v1";

// _realtime_health_check's named arguments plus B4440's deployment parameters
pub struct RealtimeProbeRequest {
    pub model: String,
    pub custom_llm_provider: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub realtime_protocol: Option<String>,
    pub model_params: JsonObject,
}

// the shared connection settings and the websocket message cap the health connection uses
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeProbeSettings {
    pub azure_realtime_protocol: Option<String>,
    pub credential_timeout_seconds: f64,
    pub max_message_size_bytes: usize,
}

// everything the probe reads outside its own types, injected so the entry builds no global
pub struct RealtimeProbeDependencies<'deps> {
    pub stored: &'deps CredentialSnapshot,
    pub metadata: &'deps dyn RealtimeModelMetadata,
    pub azure: &'deps dyn AzureAdTokens,
    pub vertex: &'deps dyn VertexProbeConfig,
    pub dialer: &'deps dyn RealtimeProbeDialer,
    pub verification: &'deps Verification,
}

// websockets.connect's arguments: the URL, the handshake headers, the SSL setting and the message cap
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeConnection {
    pub url: String,
    pub headers: Headers,
    pub ssl: RealtimeSsl,
    pub max_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProbeDialError {
    pub class: String,
    pub message: String,
    pub status: Option<u16>,
}

impl ProbeDialError {
    #[must_use]
    pub fn new(class: &str, message: String) -> Self {
        Self {
            class: class.to_owned(),
            message,
            status: None,
        }
    }
}

// one handshake, no retry: the probe returns true once the connection is open and closes it
#[async_trait]
pub trait RealtimeProbeDialer: Send + Sync {
    async fn probe(&self, connection: &ProbeConnection) -> Result<(), ProbeDialError>;
}

#[derive(Debug, Error)]
pub enum RealtimeProbeError {
    #[error("Unsupported model: {model}")]
    UnsupportedModel { model: String },
    #[error(
        "Realtime health checks are not supported for Speech-to-Text streaming model {model}; health check it with mode audio_transcription"
    )]
    VertexSpeechModel { model: String },
    #[error("the realtime health check parameter {field} must be a string")]
    ParameterType { field: &'static str },
    #[error(transparent)]
    Azure(#[from] betterlite_azure::realtime::error::RealtimeError),
    #[error(transparent)]
    OpenAi(#[from] betterlite_openai::realtime::error::RealtimeError),
    #[error(
        "Vertex AI realtime: timed out fetching Google OAuth access token after {timeout_seconds:?}s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    )]
    VertexTokenTimeout { timeout_seconds: f64 },
    #[error("{message}")]
    Token { message: String },
    #[error(transparent)]
    Dial(#[from] ProbeDialError),
}

// _realtime_health_check: resolve the merged parameters, then open the provider's connection once
pub async fn realtime_health_check(
    request: &RealtimeProbeRequest,
    settings: &RealtimeProbeSettings,
    dependencies: &RealtimeProbeDependencies<'_>,
) -> Result<(), RealtimeProbeError> {
    let merged = merge_stored_credentials(&request.model_params, dependencies.stored);
    let resolved = resolve_probe_parameters(request, &merged)?;
    if request.custom_llm_provider == AZURE_PROVIDER {
        return azure_health_check(request, settings, &merged, &resolved, dependencies).await;
    }
    let configured_base = resolved.api_base.as_deref().filter(|text| !text.is_empty());
    let default_base = match request.custom_llm_provider.as_str() {
        OPENAI_PROVIDER => OPENAI_DEFAULT_API_BASE,
        XAI_PROVIDER => XAI_DEFAULT_API_BASE,
        VERTEX_AI_PROVIDER => {
            return vertex::health_check(request, &merged, configured_base, settings, dependencies)
                .await;
        }
        _ => {
            return Err(RealtimeProbeError::UnsupportedModel {
                model: request.model.clone(),
            });
        }
    };
    let url = construct_url(
        configured_base.unwrap_or(default_base),
        &QueryParams::from([(String::from(QUERY_MODEL_KEY), Some(request.model.clone()))]),
    )?;
    probe(
        dependencies,
        url,
        bearer_headers(resolved.api_key.as_ref()),
        settings,
    )
    .await
}

// the Azure branch: AD resolution, protocol selection, the transcription intent and B545's URL and headers
async fn azure_health_check(
    request: &RealtimeProbeRequest,
    settings: &RealtimeProbeSettings,
    merged: &JsonObject,
    resolved: &ResolvedProbeParameters,
    dependencies: &RealtimeProbeDependencies<'_>,
) -> Result<(), RealtimeProbeError> {
    let ad_token = azure::ad_token(resolved.api_key.as_ref(), merged, dependencies.azure)?;
    let protocol = azure_protocol(
        request.realtime_protocol.as_deref(),
        merged.get(REALTIME_PROTOCOL_KEY),
        settings.azure_realtime_protocol.as_deref(),
    );
    let query_params = transcription_query(&request.model, dependencies.metadata);
    let version = resolved
        .api_version
        .as_deref()
        .filter(|text| !text.is_empty())
        .unwrap_or(AZURE_DEFAULT_API_VERSION);
    let connection = azure_connection(&azure::AzureProbeConnection {
        model: &request.model,
        api_key: resolved.api_key.as_ref(),
        ad_token: ad_token.as_ref(),
        api_base: resolved.api_base.as_deref(),
        api_version: version,
        protocol: &protocol,
        query_params: query_params.as_ref(),
        max_message_size_bytes: settings.max_message_size_bytes,
        verification: dependencies.verification,
    })?;
    dependencies
        .dialer
        .probe(&connection)
        .await
        .map_err(RealtimeProbeError::Dial)
}

impl From<RealtimeConnectError<VertexTokenError>> for RealtimeProbeError {
    fn from(error: RealtimeConnectError<VertexTokenError>) -> Self {
        match error {
            RealtimeConnectError::VertexTokenTimeout { timeout_seconds } => {
                Self::VertexTokenTimeout { timeout_seconds }
            }
            RealtimeConnectError::Token(token) => Self::Token {
                message: token.message,
            },
            RealtimeConnectError::Resolution(source) => Self::Token {
                message: source.to_string(),
            },
            RealtimeConnectError::UnsupportedModel { model } => Self::UnsupportedModel { model },
            RealtimeConnectError::Dispatch(source) => Self::Token {
                message: source.to_string(),
            },
            RealtimeConnectError::NotBuilt(source) => Self::Token {
                message: source.to_string(),
            },
            RealtimeConnectError::Logging { source } => Self::Token {
                message: source.to_string(),
            },
        }
    }
}

pub(crate) async fn probe(
    dependencies: &RealtimeProbeDependencies<'_>,
    url: String,
    headers: Headers,
    settings: &RealtimeProbeSettings,
) -> Result<(), RealtimeProbeError> {
    let ssl = realtime_ssl(&url, dependencies.verification);
    let connection = ProbeConnection {
        url,
        headers,
        ssl,
        max_size: settings.max_message_size_bytes,
    };
    dependencies
        .dialer
        .probe(&connection)
        .await
        .map_err(RealtimeProbeError::Dial)
}
