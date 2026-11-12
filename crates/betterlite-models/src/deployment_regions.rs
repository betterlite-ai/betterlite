use secrecy::SecretString;

use crate::resolution::{ResolutionContext, ResolutionRequest, resolve};
use error::RegionError;

pub mod error;
mod tests;

const LITELLM_PROXY: &str = "litellm_proxy";

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display, strum::EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum ModelRegion {
    Eu,
    Us,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegionLists {
    pub eu: Vec<String>,
    pub us: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderRegionLists {
    pub azure: RegionLists,
    pub vertex_ai: RegionLists,
    pub bedrock: RegionLists,
    pub watsonx: RegionLists,
}

pub struct RegionParams<'params> {
    pub model: &'params str,
    pub custom_llm_provider: Option<&'params str>,
    pub api_base: Option<&'params str>,
    pub api_key: Option<&'params SecretString>,
    pub use_litellm_proxy: bool,
    pub region_name: Option<&'params str>,
    pub vertex_location: Option<&'params str>,
    pub aws_region_name: Option<&'params str>,
    pub watsonx_region_name: Option<&'params str>,
}

impl ProviderRegionLists {
    fn for_provider(&self, provider: &str) -> Option<&RegionLists> {
        match provider {
            "azure" => Some(&self.azure),
            "vertex_ai" => Some(&self.vertex_ai),
            "bedrock" => Some(&self.bedrock),
            "watsonx" => Some(&self.watsonx),
            _ => None,
        }
    }
}

#[must_use]
pub fn get_model_region<'params>(
    custom_llm_provider: &str,
    params: &RegionParams<'params>,
    default_vertex_location: Option<&'params str>,
) -> Option<&'params str> {
    let provider_region = match custom_llm_provider {
        "vertex_ai" => params.vertex_location.or(default_vertex_location),
        "bedrock" => params.aws_region_name,
        "watsonx" => params.watsonx_region_name,
        _ => None,
    };
    provider_region.or(params.region_name)
}

pub fn infer_model_region(
    params: &RegionParams<'_>,
    context: &ResolutionContext<'_>,
    region_lists: &ProviderRegionLists,
    default_vertex_location: Option<&str>,
) -> Result<Option<ModelRegion>, RegionError> {
    let provider = if params.use_litellm_proxy {
        LITELLM_PROXY.to_owned()
    } else {
        let request = ResolutionRequest {
            model: params.model,
            custom_llm_provider: params.custom_llm_provider,
            api_base: params.api_base,
            api_key: params.api_key,
        };
        resolve(&request, context)?.provider.slug().to_owned()
    };
    let Some(region) = get_model_region(&provider, params, default_vertex_location) else {
        tracing::debug!(model = params.model, "Cannot infer model region for model");
        return Ok(None);
    };
    let Some(lists) = region_lists.for_provider(&provider) else {
        return Ok(None);
    };
    let lowered = region.to_lowercase();
    if lists.eu.iter().any(|eu| lowered.contains(eu.as_str())) {
        return Ok(Some(ModelRegion::Eu));
    }
    Ok(lists
        .us
        .iter()
        .any(|us| lowered.contains(us.as_str()))
        .then_some(ModelRegion::Us))
}
