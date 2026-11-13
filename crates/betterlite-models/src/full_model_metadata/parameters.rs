use std::collections::BTreeSet;
use std::convert::Infallible;
use std::error::Error;

use crate::identity::provider_id::ProviderId;
use crate::params::CallType;
use crate::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use crate::resolution::{ProviderIdentity, ResolutionContext, ResolutionRequest, resolve};

pub trait SupportedParameterSource: Send + Sync {
    type Error: Error + 'static;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Self::Error>;
}

// B175's supported-parameter answer for the provider a caller sends with the request.
pub struct ProviderParameterSource<'source> {
    pub resolution: &'source ResolutionContext<'source>,
    pub call_type: CallType,
    pub config: OpenAiConfigTag,
    pub capabilities: &'source BTreeSet<OpenAiCapability>,
    pub base_model: Option<ModelProjection<'source>>,
}

impl SupportedParameterSource for ProviderParameterSource<'_> {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        let detected = custom_llm_provider.map_or_else(
            || inferred_provider(model, self.resolution),
            |name| name.parse::<ProviderId>().ok(),
        );
        let Some(provider) = detected else {
            return Ok(None);
        };
        let query = CapabilityQuery {
            provider,
            call_type: self.call_type,
            config: self.config,
            model: ModelProjection {
                model,
                capabilities: self.capabilities,
            },
            base_model: self.base_model,
        };
        Ok(match supported_parameters(&query) {
            ParameterCapabilities::Listed(names) => {
                Some(names.into_iter().map(str::to_owned).collect())
            }
            ParameterCapabilities::Absent => None,
        })
    }
}

// LiteLLM infers the provider from the model when the caller names none; B174 is that resolver.
fn inferred_provider(model: &str, resolution: &ResolutionContext<'_>) -> Option<ProviderId> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    match resolve(&request, resolution).ok()?.provider {
        ProviderIdentity::Known(provider) => Some(provider),
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
    }
}
