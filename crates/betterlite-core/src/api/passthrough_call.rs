mod auth;
pub mod config;
pub mod logging;
mod mantle;
mod params;
mod send;
mod urls;
mod wire;

mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_context::resource_config_selection::passthrough::{
    PassthroughConfigTag, select_passthrough_config,
};
use betterlite_http::interactions::base::{
    CallFailure, RequestFailure, ResponseHeaders, handle_error,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::Object;
use bytes::Bytes;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use self::config::{ConfigError, Fault, PassthroughCallConfig};
use self::logging::RelayLogging;
use self::send::{Failure, run};
use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::spend_collection::OwnedProviderSpend;

// One body field of a multipart pass-through upload
#[derive(Debug)]
pub struct PassthroughUpload {
    pub field: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub bytes: Bytes,
}

// The timeouts the call resolves when the caller hands over its own client settings
#[derive(Debug, Clone, Copy)]
pub struct PassthroughClient {
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

// The arguments of litellm.allm_passthrough_route, one field per named parameter
#[derive(Debug)]
pub struct PassthroughCallArgs {
    pub method: String,
    pub endpoint: String,
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub request_query_params: Option<IndexMap<String, String>>,
    pub request_headers: Option<Vec<(String, String)>>,
    pub content: Option<Bytes>,
    pub data: Option<Object>,
    pub files: Option<Vec<PassthroughUpload>>,
    pub json: Option<JsonValue>,
    pub params: Option<Vec<(String, String)>>,
    pub cookies: Option<Vec<(String, String)>>,
    pub client: Option<PassthroughClient>,
    pub extra: Object,
}

// The B3572 relay the streaming answer is carried by
pub type PassthroughRelay =
    AsyncPassthroughStream<OwnedProviderSpend<PassthroughCallConfig>, RelayLogging>;

pub enum PassthroughBody {
    Full(Box<Bytes>),
    Stream(Box<PassthroughRelay>),
}

pub struct PassthroughReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: PassthroughBody,
}

#[derive(Debug, Error)]
pub enum PassthroughCallError {
    #[error("the upstream answered with status {status}")]
    UpstreamStatus {
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
    },
    #[error("Provider {0} not found")]
    ProviderNotFound(String),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error("{}", .0.message)]
    Mapped(Box<ProviderError>),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the pass-through call was refused with status {status}: {message}")]
    Routing { status: u16, message: String },
}

// The router's side of the call; the SDK implements it over the shared route table
pub trait PassthroughRouterCalls: Send + Sync {
    fn allm_passthrough_route(
        &self,
        args: PassthroughCallArgs,
    ) -> impl Future<Output = Result<PassthroughReply, PassthroughCallError>> + Send;
}

// litellm.allm_passthrough_route: resolve the provider, pick its config, then run the SDK call
pub async fn allm_passthrough_route(
    mut args: PassthroughCallArgs,
    logging: &mut LoggingState,
    http: &ProxiedClient,
    environment: &BTreeMap<String, String>,
) -> Result<PassthroughReply, PassthroughCallError> {
    args.extra
        .insert("allm_passthrough_route".to_owned(), JsonValue::Bool(true));
    let resolved = resolve_provider(&args, environment)?;
    let Some(tag) = config_tag(&resolved) else {
        return Err(PassthroughCallError::ProviderNotFound(
            resolved.provider.slug().to_owned(),
        ));
    };
    let shared_environment = Arc::new(environment.clone());
    let config = PassthroughCallConfig {
        tag,
        environment: Arc::clone(&shared_environment),
    };
    match run(&args, &resolved, &config, logging, http).await {
        Ok(reply) => Ok(reply),
        Err(Failure::Upstream(status)) => Err(PassthroughCallError::UpstreamStatus {
            status: status.status,
            headers: status.headers,
            body: status.body,
        }),
        Err(Failure::Fault(fault)) => Err(map_fault(&args, fault, &shared_environment)),
    }
}

// get_llm_provider: the catalog, names and credentials come from outside; the Mantle endpoint comes from the call
fn resolve_provider(
    args: &PassthroughCallArgs,
    environment: &BTreeMap<String, String>,
) -> Result<ResolvedModel, ResolutionError> {
    let catalog = ModelCatalog::default();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    resolve(
        &ResolutionRequest {
            model: &args.model,
            custom_llm_provider: args.custom_llm_provider.as_deref(),
            api_base: args.api_base.as_deref(),
            api_key: args.api_key.as_ref(),
        },
        &ResolutionContext {
            catalog: &catalog,
            known_model_names: &known_model_names,
            custom_providers: &custom_providers,
            environment,
            registry: JSONProviderRegistry::default(),
            policies: mantle::policies(args),
            globals: GlobalCredentials::default(),
        },
    )
}

// ProviderConfigManager.get_provider_passthrough_config, through B5360's tag
fn config_tag(resolved: &ResolvedModel) -> Option<PassthroughConfigTag> {
    match &resolved.provider {
        ProviderIdentity::Known(provider) => select_passthrough_config(&resolved.model, *provider),
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
    }
}

// The except block: resolve again over the same map, then let _handle_error map the original failure
fn map_fault(
    args: &PassthroughCallArgs,
    fault: Fault,
    environment: &Arc<BTreeMap<String, String>>,
) -> PassthroughCallError {
    let resolved = match resolve_provider(args, environment.as_ref()) {
        Ok(resolved) => resolved,
        Err(error) => return PassthroughCallError::Resolution(error),
    };
    let Some(tag) = config_tag(&resolved) else {
        return match fault {
            Fault::Send(error) => PassthroughCallError::Transport(*error),
            Fault::Failure(_) => {
                PassthroughCallError::ProviderNotFound(resolved.provider.slug().to_owned())
            }
        };
    };
    let (status, synthesized, message) = match fault {
        Fault::Failure(detail) => (detail.status, detail.synthesized, detail.message),
        Fault::Send(error) => (500, true, error.to_string()),
    };
    let failure = RequestFailure::Status {
        status_code: status,
        headers: ResponseHeaders::default(),
        body: message,
    };
    let config = PassthroughCallConfig {
        tag,
        environment: Arc::clone(environment),
    };
    match handle_error(&config, failure) {
        CallFailure::Provider(error) => {
            PassthroughCallError::Mapped(Box::new(mapped_error(error, synthesized)))
        }
        CallFailure::PassedThrough(problem) => {
            let error = ConfigError::ValueError {
                message: problem.to_string(),
            };
            PassthroughCallError::Mapped(Box::new(mapped_error(error, synthesized)))
        }
    }
}

// _handle_error's provider_error: ProviderError's own flag is overwritten when the status was synthesized
fn mapped_error(error: ConfigError, synthesized: bool) -> ProviderError {
    match error {
        ConfigError::Provider(mut provider) => {
            if synthesized {
                provider.status_code_is_synthesized = true;
            }
            *provider
        }
        ConfigError::Message { status, message } => provider_error(status, message, synthesized),
        ConfigError::ValueError { message } => provider_error(500, message, synthesized),
        ConfigError::Passthrough(source) => provider_error(500, source.to_string(), synthesized),
    }
}

fn provider_error(status: u16, message: String, synthesized: bool) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: status,
        message,
        headers: Some(Vec::new()),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: synthesized,
    })
}
