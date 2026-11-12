use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::anthropic_capability_catalog::capability::supports_model_capability;

// None means the effort is allowed; Some carries the message LiteLLM's callers raise as a 400.
pub async fn validate_effort_for_model<Source, Transport>(
    model: &str,
    effort: Option<&str>,
    custom_llm_provider: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Option<String>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    match effort {
        Some("max") => {
            let adaptive = supports_model_capability(
                model,
                "supports_adaptive_thinking",
                custom_llm_provider,
                environment,
                catalogs,
            )
            .await;
            let max_level = adaptive
                || supports_model_capability(
                    model,
                    "supports_max_reasoning_effort",
                    custom_llm_provider,
                    environment,
                    catalogs,
                )
                .await;
            (!max_level)
                .then(|| format!("effort='max' is not supported by this model. Got model: {model}"))
        }
        Some("xhigh") => {
            let supported = supports_model_capability(
                model,
                "supports_xhigh_reasoning_effort",
                custom_llm_provider,
                environment,
                catalogs,
            )
            .await;
            (!supported).then(|| {
                format!("effort='xhigh' is not supported by this model. Got model: {model}")
            })
        }
        Some(_) | None => None,
    }
}

// _raise_invalid_reasoning_effort always raises this BadRequestError carrying the routed model and provider.
#[derive(Debug, thiserror::Error)]
pub enum EffortError {
    #[error(
        "Invalid reasoning_effort: {value}. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'."
    )]
    InvalidReasoningEffort {
        value: String,
        model: String,
        provider: String,
    },
}

impl EffortError {
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::InvalidReasoningEffort {
                model, provider, ..
            } => {
                let context = ErrorContext {
                    message: self.to_string(),
                    model: Some(model.clone()),
                    provider: Some(provider.clone()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                };
                ErrorClass::BadRequest(Box::new(ResponseDetails::from(context)))
            }
        }
    }
}
