mod tests;
mod token;

use std::error::Error;
use std::sync::Arc;

use betterlite_azure::realtime::protocol::client_protocol;
use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_config::sections::xai_identity::Settings as XaiIdentitySettings;
use betterlite_context::resource_config_selection::realtime::{
    RealtimeConfigTag, select_realtime_config,
};
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_logging::logging_state::{LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::realtime::connect::DEFAULT_API_BASE;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_openai::xai_identity::{self, KeyOrder};
use betterlite_openai::xai_realtime::connect::XAI_API_BASE;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

use crate::api::realtime_backend_transport::dial::RealtimeWebSocketDialer;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::vertex_chirp::target::AccessTokenResolver;
use crate::dispatch::error::DispatchError;

pub use token::{
    VertexAccessTokenSource, VertexTokenError, VertexTokenFuture, VertexTokenRefresh,
    resolve_vertex_access_token_bounded,
};

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

// the transport arguments of the two branches that go through B2457
pub struct TransportArgs {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub client: Option<Arc<dyn RealtimeWebSocketDialer>>,
    pub timeout: Option<f64>,
    pub headers: HeaderMap,
    pub user_api_key_dict: Option<Json>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub query_params: Option<IndexMap<String, String>>,
}

pub struct ProviderConfigConnection<Config> {
    pub model: String,
    pub config: Arc<Config>,
    pub transport: TransportArgs,
}

pub struct VertexConnection {
    pub model: String,
    pub access_token: SecretString,
    pub refresh: Arc<dyn AccessTokenResolver>,
    pub project: String,
    pub location: Option<String>,
    pub transport: TransportArgs,
}

pub struct AzureConnection {
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: String,
    pub azure_ad_token: Option<SecretString>,
    pub protocol: String,
    pub timeout: Option<f64>,
    pub query_params: Option<QueryParams>,
    pub user_api_key_dict: Option<Json>,
    pub litellm_metadata: IndexMap<String, Json>,
}

pub struct OpenAiConnection {
    pub model: String,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub timeout: Option<f64>,
    pub query_params: Option<QueryParams>,
    pub user_api_key_dict: Option<Json>,
    pub litellm_metadata: IndexMap<String, Json>,
}

pub struct XaiConnection {
    pub model: String,
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub timeout: Option<f64>,
    pub query_params: Option<QueryParams>,
    pub user_api_key_dict: Option<Json>,
    pub litellm_metadata: IndexMap<String, Json>,
}

// bedrock forwards only these: no query, headers, principal or internal metadata
pub struct BedrockConnection {
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub timeout: Option<f64>,
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
}

pub enum RealtimeConnection<Config> {
    Provider(ProviderConfigConnection<Config>),
    Azure(AzureConnection),
    OpenAi(OpenAiConnection),
    Bedrock(BedrockConnection),
    Xai(XaiConnection),
    Vertex(VertexConnection),
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
        Err(DispatchError::NotBuilt {
            entry: "provider_realtime_config",
        }
        .into())
    }

    fn vertex_access_tokens(
        &self,
    ) -> Result<
        Arc<dyn VertexAccessTokenSource<Error = Self::Error>>,
        RealtimeConnectError<Self::Error>,
    > {
        Err(DispatchError::NotBuilt {
            entry: "vertex_access_token_resolver",
        }
        .into())
    }

    fn azure_ad_token(
        &self,
        _request: AzureAdTokenRequest<'_>,
    ) -> Result<Option<SecretString>, RealtimeConnectError<Self::Error>> {
        Err(DispatchError::NotBuilt {
            entry: "azure_ad_token_credentials",
        }
        .into())
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
    Token(OperationError),
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

// Everything the branches share: the resolved identity, the merged headers, the copied query, the metadata and the logging update.
pub async fn arealtime<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    logging: &mut LoggingState,
    resolution: &ResolutionContext<'_>,
    call: &RealtimeConnectCall<'_>,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let prepared = prepare(logging, resolution, call)?;
    connect(dispatch, settings, call, &prepared).await
}

struct Prepared {
    model: String,
    provider: ProviderIdentity,
    dynamic_api_base: Option<String>,
    dynamic_api_key: Option<SecretString>,
    headers: HeaderMap,
    query_params: Option<IndexMap<String, String>>,
    metadata: IndexMap<String, Json>,
}

fn prepare<OperationError>(
    logging: &mut LoggingState,
    resolution: &ResolutionContext<'_>,
    call: &RealtimeConnectCall<'_>,
) -> Result<Prepared, RealtimeConnectError<OperationError>>
where
    OperationError: Error + 'static,
{
    let request = ResolutionRequest {
        model: call.model,
        custom_llm_provider: None,
        api_base: call.params.api_base.as_deref(),
        api_key: call.params.api_key.as_ref(),
    };
    let resolved = resolve(&request, resolution)?;
    let mut headers = call.params.headers.clone().unwrap_or_default();
    if let Some(extra) = &call.params.extra_headers {
        headers.extend(extra.clone());
    }
    let mut query_params = call.query_params.cloned();
    if let Some(query) = query_params.as_mut()
        && query.contains_key(QUERY_MODEL_KEY)
    {
        query.insert(String::from(QUERY_MODEL_KEY), resolved.model.clone());
    }
    let metadata = build_litellm_metadata(
        call.params.litellm_metadata.as_ref(),
        call.params.metadata.as_ref(),
        call.params.guardrails.as_ref(),
    );
    log_call(logging, call, &resolved)?;
    Ok(Prepared {
        model: resolved.model,
        provider: resolved.provider,
        dynamic_api_base: resolved.api_base,
        dynamic_api_key: resolved.api_key,
        headers,
        query_params,
        metadata,
    })
}

// LiteLLM merges the kwargs metadata and litellm_metadata for update_from_kwargs and adds the call type to the params
fn log_call<OperationError>(
    logging: &mut LoggingState,
    call: &RealtimeConnectCall<'_>,
    resolved: &ResolvedModel,
) -> Result<(), RealtimeConnectError<OperationError>>
where
    OperationError: Error + 'static,
{
    let mut kwargs: IndexMap<String, Json> = IndexMap::new();
    if let Some(metadata) = &call.params.metadata {
        kwargs.insert(String::from("metadata"), Json::Object(metadata.clone()));
    }
    if let Some(metadata) = &call.params.litellm_metadata {
        kwargs.insert(
            String::from("litellm_metadata"),
            Json::Object(metadata.clone()),
        );
    }
    let mut litellm_params = call.litellm_params.clone();
    litellm_params.insert(String::from("_arealtime"), Json::Bool(true));
    let mut additional_params: IndexMap<String, Json> = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.slug().to_owned()),
    );
    Ok(update_from_kwargs(
        logging,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: call.params.user.clone(),
            optional_params: IndexMap::new(),
            additional_params,
        },
    )?)
}

#[must_use]
pub fn build_litellm_metadata(
    litellm_metadata: Option<&IndexMap<String, Json>>,
    metadata: Option<&IndexMap<String, Json>>,
    guardrails: Option<&Json>,
) -> IndexMap<String, Json> {
    let mut built = litellm_metadata.cloned().unwrap_or_default();
    let from_metadata = metadata.and_then(|metadata| metadata.get(GUARDRAILS_KEY));
    let selected = from_metadata
        .filter(|value| !empty_json(value))
        .or_else(|| guardrails.filter(|value| !empty_json(value)));
    if let Some(guardrails) = selected {
        built.insert(String::from(GUARDRAILS_KEY), guardrails.clone());
    }
    built
}

// Python's `or` over the credential and location slots: an empty value falls through
fn empty_json(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
        Json::Bool(_) | Json::Number(_) => false,
    }
}

async fn connect<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    if let ProviderIdentity::Known(provider) = &prepared.provider
        && let Some(tag) = select_realtime_config(&prepared.model, *provider)
    {
        return provider_connection(dispatch, call, prepared, tag);
    }
    match &prepared.provider {
        ProviderIdentity::Known(ProviderId::Azure | ProviderId::AzureAi) => Ok(
            RealtimeConnection::Azure(azure_connection(dispatch, settings, call, prepared)?),
        ),
        ProviderIdentity::Known(ProviderId::Openai) => Ok(RealtimeConnection::OpenAi(
            openai_connection(settings, call, prepared),
        )),
        ProviderIdentity::Known(ProviderId::Bedrock) => Ok(RealtimeConnection::Bedrock(
            bedrock_connection(call, prepared),
        )),
        ProviderIdentity::Known(ProviderId::Xai) => Ok(RealtimeConnection::Xai(xai_connection(
            settings, call, prepared,
        ))),
        ProviderIdentity::Known(ProviderId::VertexAi) => Ok(RealtimeConnection::Vertex(
            vertex_connection(dispatch, settings, call, prepared).await?,
        )),
        ProviderIdentity::Known(_unsupported) => Err(unsupported(prepared)),
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_name) => Err(unsupported(prepared)),
    }
}

fn unsupported<OperationError>(prepared: &Prepared) -> RealtimeConnectError<OperationError> {
    RealtimeConnectError::UnsupportedModel {
        model: prepared.model.clone(),
    }
}

// generic configs receive the original named api_base and api_key, never the resolver's replacements
fn provider_connection<Dispatch>(
    dispatch: &Dispatch,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
    tag: RealtimeConfigTag,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let config = dispatch.provider_config(tag)?;
    Ok(RealtimeConnection::Provider(ProviderConfigConnection {
        model: prepared.model.clone(),
        config,
        transport: transport(
            call,
            prepared,
            call.params.api_base.clone(),
            call.params.api_key.clone(),
        ),
    }))
}

fn transport(
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
    api_base: Option<String>,
    api_key: Option<SecretString>,
) -> TransportArgs {
    TransportArgs {
        api_base,
        api_key,
        client: call.client.clone(),
        timeout: call.params.timeout,
        headers: prepared.headers.clone(),
        user_api_key_dict: call.params.user_api_key_dict.clone(),
        litellm_metadata: prepared.metadata.clone(),
        query_params: prepared.query_params.clone(),
    }
}

struct BranchCredentials {
    api_base: Option<String>,
    api_key: Option<SecretString>,
}

fn azure_credentials(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> BranchCredentials {
    BranchCredentials {
        api_base: first_text([
            prepared.dynamic_api_base.as_deref(),
            call.params.generic_api_base.as_deref(),
            settings.api_base.as_deref(),
            settings.azure_api_base.as_deref(),
        ]),
        api_key: first_secret([
            prepared.dynamic_api_key.as_ref(),
            settings.api_key.as_ref(),
            settings.openai_key.as_ref(),
            settings.azure_api_key.as_ref(),
        ]),
    }
}

fn azure_connection<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<AzureConnection, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let credentials = azure_credentials(settings, call, prepared);
    let api_version = first_text([
        call.params.api_version.as_deref(),
        call.params.generic_api_version.as_deref(),
    ])
    .unwrap_or_else(|| String::from(AZURE_DEFAULT_API_VERSION));
    let query_params = connector_query(prepared.query_params.as_ref());
    let configured = first_text([
        call.params.realtime_protocol.as_deref(),
        call.params.generic_realtime_protocol.as_deref(),
        settings.azure_realtime_protocol.as_deref(),
    ]);
    let protocol = client_protocol(
        configured.as_deref(),
        query_params.as_ref(),
        call.client_sent_beta,
    );
    // a truthy key suppresses the Azure AD lookup entirely
    let azure_ad_token = if credentials.api_key.is_none() {
        dispatch.azure_ad_token(AzureAdTokenRequest {
            azure_ad_token: call.params.azure_ad_token.as_ref(),
            params: call.params,
        })?
    } else {
        None
    };
    Ok(AzureConnection {
        model: prepared.model.clone(),
        api_base: credentials.api_base,
        api_key: credentials.api_key,
        api_version,
        azure_ad_token,
        protocol,
        timeout: call.params.timeout,
        query_params,
        user_api_key_dict: call.params.user_api_key_dict.clone(),
        litellm_metadata: prepared.metadata.clone(),
    })
}

fn openai_credentials(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> (String, Option<SecretString>) {
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        call.params.generic_api_base.as_deref(),
        settings.api_base.as_deref(),
    ])
    .unwrap_or_else(|| String::from(DEFAULT_API_BASE));
    let api_key = first_secret([
        prepared.dynamic_api_key.as_ref(),
        settings.api_key.as_ref(),
        settings.openai_key.as_ref(),
        settings.openai_api_key.as_ref(),
    ]);
    (api_base, api_key)
}

fn openai_connection(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> OpenAiConnection {
    let (api_base, api_key) = openai_credentials(settings, call, prepared);
    OpenAiConnection {
        model: prepared.model.clone(),
        api_base,
        api_key,
        timeout: call.params.timeout,
        query_params: connector_query(prepared.query_params.as_ref()),
        user_api_key_dict: call.params.user_api_key_dict.clone(),
        litellm_metadata: prepared.metadata.clone(),
    }
}

fn xai_connection(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> XaiConnection {
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        call.params.generic_api_base.as_deref(),
        settings.xai_api_base.as_deref(),
    ])
    .unwrap_or_else(|| String::from(XAI_API_BASE));
    let api_key = xai_identity::get_api_key(
        prepared.dynamic_api_key.as_ref(),
        KeyOrder::GenericBeforeEnvironment,
        &XaiIdentitySettings {
            xai_key: settings.xai_key.clone(),
            generic_api_key: settings.api_key.clone(),
            environment_key: settings.xai_api_key.clone(),
            api_base: settings.xai_api_base.clone(),
        },
    );
    XaiConnection {
        model: prepared.model.clone(),
        api_base,
        api_key,
        timeout: call.params.timeout,
        query_params: connector_query(prepared.query_params.as_ref()),
        user_api_key_dict: call.params.user_api_key_dict.clone(),
        litellm_metadata: prepared.metadata.clone(),
    }
}

fn bedrock_connection(call: &RealtimeConnectCall<'_>, prepared: &Prepared) -> BedrockConnection {
    let params = call.params;
    // bedrock reads the named api_base and api_key, with no generic or global fallback
    BedrockConnection {
        model: prepared.model.clone(),
        api_base: first_text([
            prepared.dynamic_api_base.as_deref(),
            params.api_base.as_deref(),
        ]),
        api_key: first_secret([prepared.dynamic_api_key.as_ref(), params.api_key.as_ref()]),
        timeout: params.timeout,
        aws_region_name: params.aws_region_name.clone(),
        aws_access_key_id: params.aws_access_key_id.clone(),
        aws_secret_access_key: params.aws_secret_access_key.clone(),
        aws_session_token: params.aws_session_token.clone(),
        aws_role_name: params.aws_role_name.clone(),
        aws_session_name: params.aws_session_name.clone(),
        aws_profile_name: params.aws_profile_name.clone(),
        aws_web_identity_token: params.aws_web_identity_token.clone(),
        aws_sts_endpoint: params.aws_sts_endpoint.clone(),
        aws_bedrock_runtime_endpoint: params.aws_bedrock_runtime_endpoint.clone(),
        aws_external_id: params.aws_external_id.clone(),
        aws_session_tags: params.aws_session_tags.clone(),
    }
}

fn vertex_credentials(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> Option<Json> {
    let named = [
        call.params.vertex_credentials.as_ref(),
        call.params.vertex_ai_credentials.as_ref(),
    ];
    named
        .into_iter()
        .flatten()
        .find(|value| !empty_json(value))
        .cloned()
        .or_else(|| {
            settings
                .vertexai_credentials
                .as_ref()
                .filter(|value| !empty_json(value))
                .cloned()
        })
}

fn vertex_project(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> Option<String> {
    first_text([
        call.params.vertex_project.as_deref(),
        call.params.vertex_ai_project.as_deref(),
        settings.vertex_project.as_deref(),
        settings.vertexai_project.as_deref(),
    ])
}

fn vertex_location(
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
) -> Option<String> {
    first_text([
        call.params.vertex_location.as_deref(),
        call.params.vertex_ai_location.as_deref(),
        settings.vertex_location.as_deref(),
        settings.vertexai_location.as_deref(),
    ])
}

async fn vertex_connection<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<VertexConnection, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let credentials = vertex_credentials(settings, call);
    let project = vertex_project(settings, call);
    let location = vertex_location(settings, call);
    let source = dispatch.vertex_access_tokens()?;
    let (access_token, resolved_project) = resolve_vertex_access_token_bounded(
        source.as_ref(),
        credentials.as_ref(),
        project.as_deref(),
        settings.credential_timeout_seconds,
    )
    .await?;
    // the refresh callback keeps the first call's credentials and the project that call resolved
    let refresh: Arc<dyn AccessTokenResolver> = Arc::new(VertexTokenRefresh::new(
        Arc::clone(&source),
        credentials,
        resolved_project.clone(),
        settings.credential_timeout_seconds,
    ));
    let api_base = first_text([
        prepared.dynamic_api_base.as_deref(),
        call.params.generic_api_base.as_deref(),
    ]);
    Ok(VertexConnection {
        model: prepared.model.clone(),
        access_token,
        refresh,
        project: resolved_project,
        location,
        transport: TransportArgs {
            api_base,
            api_key: None,
            client: call.client.clone(),
            timeout: call.params.timeout,
            headers: prepared.headers.clone(),
            user_api_key_dict: call.params.user_api_key_dict.clone(),
            litellm_metadata: prepared.metadata.clone(),
            query_params: prepared.query_params.clone(),
        },
    })
}

// B528 and B545 read Option-valued query parameters, unlike B2457's string map
fn connector_query(query: Option<&IndexMap<String, String>>) -> Option<QueryParams> {
    query.map(|query| {
        query
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect::<QueryParams>()
    })
}

fn first_text<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}
