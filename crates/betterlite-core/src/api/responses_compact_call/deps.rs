use betterlite_config::loader::EnvironmentLookup;

use crate::api::responses_compact_http::config::CompactConfig;
use crate::api::responses_compact_http::service::ClientSource;
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::ModelResolver;

use super::error::CompactParamError;

// What the factory needs to build a provider: the resolved model, its provider and the caller's base
pub struct ConfigRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub api_base: Option<&'request str>,
}

// B157 decides whether a provider has a responses config; the factory builds the object both read
pub trait CompactConfigFactory {
    type Provider: CompactConfig + ResponsesParamProvider<Error = CompactParamError>;

    fn compact_config(&self, request: &ConfigRequest<'_>) -> Option<Self::Provider>;
}

pub struct CompactResponsesDeps<'deps, S: ClientSource, F: CompactConfigFactory> {
    pub resolver: &'deps dyn ModelResolver,
    pub lookup: &'deps dyn ConfigLookup,
    pub configs: &'deps F,
    pub source: &'deps S,
    pub environment: &'deps dyn EnvironmentLookup,
    pub drop_params: bool,
}
