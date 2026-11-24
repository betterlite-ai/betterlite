use std::error::Error;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest};
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use betterlite_openai::credentials::LiteLlmGlobals;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use self::resolve::resolve_call;
use super::error::WsNativeError;
use super::logging::WsNativeLogging;
use super::request::{build_litellm_metadata_for_ws, routed_first_frame, ws_request_defaults};
use crate::api::responses_provider_selection::provider_config::{ConfigLookup, ResponsesApiConfig};
use crate::api::responses_provider_selection::resolution::ModelResolver;

pub mod resolve;

pub const RESPONSES_WS_ROUTING_HINT_KEYS: [&str; 2] = ["input", "previous_response_id"];
const EXPLICIT_KEYS: [&str; 12] = [
    "user_api_key_dict",
    "litellm_metadata",
    "custom_llm_provider",
    "model",
    "websocket",
    "litellm_logging_obj",
    "api_base",
    "api_key",
    "timeout",
    "first_message",
    "input",
    "previous_response_id",
];

pub struct WsResolved {
    pub model: String,
    pub provider: Option<String>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

pub trait NativeWsResolution: Send + Sync {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<WsResolved, ResolutionError>;
}

impl<T> NativeWsResolution for T
where
    T: ModelResolver + Send + Sync,
{
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<WsResolved, ResolutionError> {
        let outcome = self.resolve_model(request)?;
        Ok(WsResolved {
            model: outcome.model,
            provider: Some(outcome.provider.slug().to_owned()),
            api_base: outcome.api_base,
            api_key: outcome.api_key,
        })
    }
}

pub struct NativeWsDeps<'deps, Handler> {
    pub resolution: &'deps dyn NativeWsResolution,
    pub configs: &'deps (dyn ConfigLookup + Sync),
    pub logging: &'deps dyn WsNativeLogging,
    pub globals: &'deps LiteLlmGlobals,
    pub openai_api_key: Option<&'deps SecretString>,
    pub auto_summary: bool,
    pub handler: &'deps Handler,
}

pub struct NativeWsCall<'call, Client> {
    pub model: &'call str,
    pub websocket: &'call Client,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub timeout: Option<f64>,
    pub user_api_key_dict: Option<&'call UserAPIKeyAuth>,
    pub kwargs: &'call IndexMap<String, Json>,
}

pub struct ProviderWsRequest<'request, Client> {
    pub model: &'request str,
    pub websocket: &'request Client,
    pub logging: &'request dyn WsNativeLogging,
    pub provider_config: Option<ResponsesApiConfig>,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub timeout: Option<f64>,
    pub first_message: Option<String>,
    pub user_api_key_dict: Option<&'request UserAPIKeyAuth>,
    pub litellm_metadata: IndexMap<String, Json>,
    pub custom_llm_provider: Option<&'request str>,
    pub request_defaults: WsRequestDefaults<Json>,
    pub remaining_kwargs: IndexMap<String, Json>,
}

pub trait NativeWsProviderHandler<Client>: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    fn responses_websocket<'call>(
        &'call self,
        request: ProviderWsRequest<'call, Client>,
    ) -> impl Future<Output = Result<Option<ErrorClass>, Self::Error>> + Send + 'call;
}

#[derive(Debug, Error)]
pub enum NativeWsCallError<HandlerError>
where
    HandlerError: Error + 'static,
{
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Request(#[from] WsNativeError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(HandlerError),
}

pub async fn responses_websocket<Client, Handler>(
    deps: &NativeWsDeps<'_, Handler>,
    call: NativeWsCall<'_, Client>,
) -> Result<Option<ErrorClass>, NativeWsCallError<Handler::Error>>
where
    Client: Sync,
    Handler: NativeWsProviderHandler<Client>,
{
    let resolved = resolve_call(deps, &call)?;
    let deployment_kwargs = without_keys(call.kwargs, &RESPONSES_WS_ROUTING_HINT_KEYS);
    let remaining_kwargs = without_keys(call.kwargs, &EXPLICIT_KEYS);
    let routed_input = call
        .kwargs
        .get("input")
        .filter(|value| !matches!(**value, Json::Null));
    let first_message = match call.kwargs.get("first_message") {
        Some(Json::Text(text)) => Some(routed_first_frame(text, routed_input)),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    };
    let request = ProviderWsRequest {
        model: &resolved.model,
        websocket: call.websocket,
        logging: deps.logging,
        provider_config: resolved.provider_config,
        api_base: resolved.api_base.as_deref(),
        api_key: resolved.api_key.as_ref(),
        timeout: call.timeout,
        first_message,
        user_api_key_dict: call.user_api_key_dict,
        litellm_metadata: build_litellm_metadata_for_ws(call.kwargs)?,
        custom_llm_provider: resolved.provider.as_deref(),
        request_defaults: ws_request_defaults(&deployment_kwargs, deps.auto_summary)?,
        remaining_kwargs,
    };
    deps.handler
        .responses_websocket(request)
        .await
        .map_err(NativeWsCallError::Handler)
}

fn without_keys(kwargs: &IndexMap<String, Json>, excluded: &[&str]) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .filter(|(name, _value)| !excluded.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
