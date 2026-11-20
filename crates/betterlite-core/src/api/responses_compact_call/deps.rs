use betterlite_config::loader::EnvironmentLookup;

use crate::api::responses_compact_http::config::CompactConfig;
use crate::api::responses_compact_http::service::ClientSource;
use crate::api::responses_parameters::provider::ResponsesParamProvider;
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::ModelResolver;

use super::error::CompactParamError;

// B157 decides whether a provider has a responses config; the factory builds the one object the
// compact handler and the parameter filter both read
pub trait CompactConfigFactory {
    type Provider: CompactConfig + ResponsesParamProvider<Error = CompactParamError>;

    fn compact_config(
        &self,
        model: &str,
        provider: &str,
        api_base: Option<&str>,
    ) -> Option<Self::Provider>;
}

pub struct CompactResponsesDeps<'deps, S: ClientSource, F: CompactConfigFactory> {
    pub resolver: &'deps dyn ModelResolver,
    pub lookup: &'deps dyn ConfigLookup,
    pub configs: &'deps F,
    pub source: &'deps S,
    pub environment: &'deps dyn EnvironmentLookup,
    pub drop_params: bool,
}
