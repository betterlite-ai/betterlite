use std::num::ParseIntError;

use betterlite_models::headers::affinity::AffinityError;

use crate::deployment_pricing::error::DeploymentPricingError;

// Each variant is a ValueError or BadRequestError that LiteLLM raises while it builds or registers one deployment.
#[derive(Debug, thiserror::Error)]
pub enum InvalidDeployment {
    #[error("litellm_params.model is missing or not a string")]
    MissingModel,
    #[error("{field} is not a string")]
    NotText { field: &'static str },
    #[error("max_retries {value:?} is not an integer")]
    MaxRetries {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error(transparent)]
    AffinityHeader(#[from] AffinityError),
    #[error("model_info.id must be a string or an integer")]
    ModelInfoId,
    #[error("{message}")]
    Ptu { message: String },
    #[error("{message}")]
    Rejected { message: String },
    #[error("{message}")]
    ProviderUnresolved { message: String },
    #[error("tag_regex must be a list of strings")]
    TagRegexType,
    #[error("Invalid regex in tag_regex for model '{model_name}': {pattern:?}")]
    TagRegex {
        model_name: String,
        pattern: String,
        #[source]
        source: Box<fancy_regex::Error>,
    },
    #[error(
        "vertex_project, and vertex_location must be set in litellm_params for pass-through endpoints."
    )]
    VertexLocation,
    #[error("a deployment field does not read as JSON")]
    Wire(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentError<Fetch> {
    #[error(transparent)]
    Invalid(#[from] InvalidDeployment),
    #[error(transparent)]
    Pricing(#[from] DeploymentPricingError<Fetch>),
}
