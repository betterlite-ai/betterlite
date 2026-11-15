use std::error::Error;
use std::future::Future;

use betterlite_models::full_model_metadata::record::ModelInfo;

pub mod catalog_flags;
pub mod effort;
pub mod endpoints;
pub mod mantle;
pub mod strict_tools;
pub mod tool_search;

mod tests;

const MODEL_KEY_FIELD: &str = "key";

#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("model map entry {model} is not an object")]
    MalformedEntry { model: String },
    #[error("{field} has a value membership cannot be checked against")]
    NotIterable { field: &'static str },
    #[error("output_config.effort is not a hashable value")]
    UnhashableEffort,
    #[error("model name pattern is invalid")]
    Pattern(#[from] fancy_regex::Error),
}

// B2225's full lookup answers this, supported-parameter query and provider overlay included.
pub trait ModelInfoLookup: Sync {
    type Error: Error + Send + Sync + 'static;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Self::Error>> + Send;
}
