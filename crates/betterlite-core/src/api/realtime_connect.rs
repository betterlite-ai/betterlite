mod branches;
pub mod dispatch;
mod tests;
pub mod token;

use std::error::Error;
use std::sync::Arc;

use betterlite_context::resource_config_selection::realtime::RealtimeConfigTag;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::realtime::url::QueryParams;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::api::realtime_backend_transport::dial::RealtimeWebSocketDialer;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::vertex_chirp::target::AccessTokenResolver;
use crate::dispatch::error::DispatchError;

pub const AZURE_DEFAULT_API_VERSION: &str = "2024-10-01-preview";
pub const QUERY_MODEL_KEY: &str = "model";
pub const GUARDRAILS_KEY: &str = "guardrails";

// _arealtime's named arguments and the kwargs each branch reads, all optional as in the source
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealtimeConnectParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub timeout: Option<f64>,
    pub generic_api_base: Option<String>,
    pub generic_api_version: Option<String>,
    pub generic_realtime_protocol: Option<String>,
    pub headers: Option<HeaderMap>,
    pub extra_headers: Option<HeaderMap>,
    pub user: Option<String>,
    pub metadata: Option<IndexMap<String, Json>>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub guardrails: Option<Json>,
    pub user_api_key_dict: Option<Json>,
    pub realtime_protocol: Option<String>,
    pub vertex_credentials: Option<Json>,
    pub vertex_ai_credentials: Option<Json>,
    pub vertex_project: Option<String>,
    pub vertex_ai_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_ai_location: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_session_tags: Option<Json>,
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub azure_username: Option<String>,
    pub azure_password: Option<SecretString>,
    pub azure_scope: Option<String>,
}

pub struct RealtimeConnectCall<'call> {
    pub model: &'call str,
    pub params: &'call RealtimeConnectParams,
    pub query_params: Option<&'call IndexMap<String, String>>,
    pub client: Option<Arc<dyn RealtimeWebSocketDialer>>,
    pub client_sent_beta: bool,
    // B176's projection of get_litellm_params, which this call adds _arealtime to
    pub litellm_params: &'call IndexMap<String, Json>,
}

// what every branch hands its connection: the shared call arguments plus that branch's own extras
pub struct ConnectionArgs {
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<f64>,
    pub headers: Option<HeaderMap>,
    pub client: Option<Arc<dyn RealtimeWebSocketDialer>>,
    pub user_api_key_dict: Option<Json>,
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    pub query_params: ConnectionQuery,
}

// B2457 carries string pairs, while B528 and B545 read Option-valued parameters
pub enum ConnectionQuery {
    Transport(Option<IndexMap<String, String>>),
    Connector(Option<QueryParams>),
    Omitted,
}

pub enum Branch<Config> {
    ProviderConfig {
        config: Arc<Config>,
        tag: RealtimeConfigTag,
    },
    Azure {
        api_version: String,
        protocol: String,
        azure_ad_token: Option<SecretString>,
    },
    OpenAi,
    Bedrock(Box<RealtimeConnectParams>),
    Xai,
    Vertex {
        access_token: SecretString,
        refresh: Arc<dyn AccessTokenResolver>,
        project: String,
        location: Option<String>,
    },
}

pub struct RealtimeConnection<Config> {
    pub args: ConnectionArgs,
    pub branch: Branch<Config>,
}

pub struct AzureAdTokenRequest<'request> {
    pub azure_ad_token: Option<&'request SecretString>,
    pub params: &'request RealtimeConnectParams,
}

// The downstream operations a branch needs; every default answers NotBuilt so main's silence stays an error.
pub trait RealtimeConnectDispatch: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type ProviderConfig: BaseRealtimeConfig + 'static;

    fn provider_config(
        &self,
        _tag: RealtimeConfigTag,
    ) -> Result<Arc<Self::ProviderConfig>, RealtimeConnectError<Self::Error>> {
        Err(RealtimeConnectError::NotBuilt(DispatchError::NotBuilt {
            entry: "provider_realtime_config",
        }))
    }

    fn vertex_access_tokens(
        &self,
    ) -> Result<
        Arc<dyn VertexAccessTokenSource<Error = Self::Error>>,
        RealtimeConnectError<Self::Error>,
    > {
        Err(RealtimeConnectError::NotBuilt(DispatchError::NotBuilt {
            entry: "vertex_access_token_resolver",
        }))
    }

    fn azure_ad_token(
        &self,
        _request: AzureAdTokenRequest<'_>,
    ) -> Result<Option<SecretString>, RealtimeConnectError<Self::Error>> {
        Err(RealtimeConnectError::NotBuilt(DispatchError::NotBuilt {
            entry: "azure_ad_token_credentials",
        }))
    }
}

#[derive(Debug, Error)]
pub enum RealtimeConnectError<OperationError>
where
    OperationError: Error + 'static,
{
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("Unsupported model: {model}")]
    UnsupportedModel { model: String },
    #[error(
        "Vertex AI realtime: timed out fetching Google OAuth access token after {timeout_seconds:?}s; check network egress from the proxy to the OAuth token endpoint (oauth2.googleapis.com)"
    )]
    VertexTokenTimeout { timeout_seconds: f64 },
    #[error(transparent)]
    Token(token::VertexTokenError),
    #[error(transparent)]
    Dispatch(OperationError),
    #[error(transparent)]
    NotBuilt(#[from] DispatchError),
    #[error("the realtime logging update failed")]
    Logging {
        #[from]
        source: LoggingStateError,
    },
}
