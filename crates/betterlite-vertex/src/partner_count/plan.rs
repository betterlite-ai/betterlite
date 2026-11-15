use betterlite_config::sections::vertex_credential_sources::VertexSettings;

use crate::credential_sources::getters::VertexParams;
use crate::model_urls::base_url::get_vertex_base_url;

use super::error::PartnerCountError;
use super::options::PartnerCountOptions;

// every partner model counts tokens in us-east5 unless a location was configured
pub const DEFAULT_COUNT_TOKENS_LOCATION: &str = "us-east5";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartnerPublisher {
    Anthropic,
    MistralAi,
    Meta,
}

impl PartnerPublisher {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::MistralAi => "mistralai",
            Self::Meta => "meta",
        }
    }
}

pub fn publisher_for_model(model: &str) -> Result<PartnerPublisher, PartnerCountError> {
    if model.contains("claude") {
        Ok(PartnerPublisher::Anthropic)
    } else if model.contains("mistral") || model.contains("codestral") {
        Ok(PartnerPublisher::MistralAi)
    } else if model.contains("llama") || model.contains("meta/") {
        Ok(PartnerPublisher::Meta)
    } else {
        Err(PartnerCountError::UnknownPublisher {
            model: model.to_owned(),
        })
    }
}

// LiteLLM takes the part before the first @ and returns a model without one unchanged
#[must_use]
pub fn strip_version_suffix(model: &str) -> &str {
    match model.split_once('@') {
        Some((base, _version)) => base,
        None => model,
    }
}

pub fn build_count_tokens_endpoint(
    model: &str,
    project_id: &str,
    location: &str,
    api_base: Option<&str>,
) -> Result<String, PartnerCountError> {
    let publisher = publisher_for_model(model)?;
    // LiteLLM uses a supplied api_base as given, with no path graft, and reads an empty one as absent
    let base = match api_base.filter(|base| !base.is_empty()) {
        Some(base) => base.to_owned(),
        None => get_vertex_base_url(Some(location)).map_err(PartnerCountError::VertexUrl)?,
    };
    Ok(format!(
        "{base}/v1/projects/{project_id}/locations/{location}/publishers/{}/models/count-tokens:rawPredict",
        publisher.as_str()
    ))
}

// LiteLLM prefers the count-tokens location, then the deployment location, then us-east5 for every model
#[must_use]
pub fn count_tokens_location<'location>(
    options: &'location PartnerCountOptions,
    params: VertexParams<'location>,
    settings: &'location VertexSettings,
) -> &'location str {
    options
        .vertex_count_tokens_location
        .as_deref()
        .filter(|location| !location.is_empty())
        .or_else(|| params.safe_get_vertex_ai_location(settings))
        .unwrap_or(DEFAULT_COUNT_TOKENS_LOCATION)
}
