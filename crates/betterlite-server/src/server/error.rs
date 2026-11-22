use std::sync::Arc;

use betterlite_config::tls::TlsPolicyError;
use betterlite_config::types::RoutingStrategy;
use betterlite_env::EnvError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_openai::chat::deployment::DeploymentError;
use betterlite_provider::anthropic_dispatch::gateway::runtime::RuntimeError;
use betterlite_redact::error::RedactionError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum ChatCallsError {
    #[error("the token counter limits are invalid: {source}")]
    Counter(#[from] CountError),
    #[error("the Anthropic runtime could not be built: {source}")]
    Runtime(#[from] Arc<RuntimeError>),
}

#[derive(Debug, Error)]
pub enum ChatSetupError {
    #[error(transparent)]
    Environment {
        #[from]
        source: EnvError,
    },
    #[error(transparent)]
    Deployment {
        #[from]
        source: DeploymentError,
    },
    #[error(transparent)]
    Tls {
        #[from]
        source: TlsPolicyError,
    },
    #[error(
        "model_list entry {model_name} has an api_base the aiohttp_openai provider cannot use: {source}"
    )]
    AiohttpBase {
        model_name: String,
        #[source]
        source: ParseError,
    },
    #[error(
        "model_list entry {model_name} has an api_base the anthropic provider cannot use: {source}"
    )]
    AnthropicBase {
        model_name: String,
        #[source]
        source: ParseError,
    },
    #[error(
        "model_list entry {model_name} sets no {field}: LiteLLM's global key and base chain for {provider} is not built yet"
    )]
    UnbuiltChain {
        model_name: String,
        provider: &'static str,
        field: &'static str,
    },
    #[error(
        "router_settings.routing_strategy {strategy:?} is not supported, only simple-shuffle is"
    )]
    RoutingStrategy { strategy: RoutingStrategy },
    #[error("cannot build the outbound HTTP client: {source}")]
    Client {
        #[from]
        source: PoolError,
    },
    #[error(transparent)]
    Redactor {
        #[from]
        source: RedactionError,
    },
}
