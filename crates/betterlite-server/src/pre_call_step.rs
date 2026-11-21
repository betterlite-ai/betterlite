mod call_logging;
pub mod fallbacks;
mod guardrail_phase;
mod json;
mod logging;
mod request_setup;
mod router_branch;
mod standalone;
pub mod step;
mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::fallback_errors::FallbackErrors;
use betterlite_config::sections::pre_call::Settings as PreCallSection;
use betterlite_core::api::request_destination::model::ModelDefaults;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::secret::Redactor;
use http::request::Parts;

use crate::pre_call::failure::{PreCallError, PreCallFailure, ProxyFailure};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::PreCallPorts;
use crate::pre_call::rewrites::CliOverrides;
use crate::pre_call::route_type::RouteType;

// everything a route knows about the request before the step runs; data is the body the route built
pub struct PreCallInput<'input, K> {
    pub data: JsonMap,
    pub request: &'input Parts,
    pub key: &'input K,
    pub route_type: RouteType,
    pub version: Option<&'input str>,
    pub model: Option<&'input str>,
    pub skip_guardrails: bool,
}

pub struct PreCallOutput {
    pub data: JsonMap,
    pub logging: LoggingState,
}

// routes hold the step as a trait object, so a test can swap in a fake
#[async_trait]
pub trait PreCall<K: PreCallKey>: Send + Sync {
    async fn pre_call(&self, input: PreCallInput<'_, K>) -> Result<PreCallOutput, PreCallError>;
}

#[derive(Debug, Clone, Default)]
pub struct PreCallSettings {
    pub fallback_errors: FallbackErrors,
    pub pre_call: PreCallSection,
    pub overrides: CliOverrides,
    pub user_model: Option<String>,
    pub model_defaults: ModelDefaults,
}

pub struct PreCallStep<K: PreCallKey> {
    ports: PreCallPorts<K>,
    settings: PreCallSettings,
    loggers: Arc<dyn LoggerResolver>,
    // a bad pattern is a programming error, so the message waits here until the first call reports it
    redactor: Result<Redactor, String>,
}

fn internal(message: String) -> PreCallFailure {
    PreCallFailure::Proxy(ProxyFailure {
        message,
        error_type: ProxyErrorTypes::InternalServerError.to_string(),
        param: None,
        code: 500,
    })
}
