use thiserror::Error;

use crate::catalog_generalizations::python_text::python_repr;

#[derive(Debug, Error)]
pub enum ModelsError {
    #[error("the generalization rule list is {size} bytes, over the 1048576 byte limit")]
    RulesTooLarge { size: usize },
    #[error("the generalization rule list is not valid JSON")]
    RulesJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "LiteLLM: skipping malformed fallback generalization rule {label} (needs string 'pattern' and dict 'model_info')."
    )]
    MalformedRule { label: String },
    #[error(
        "LiteLLM: skipping fallback generalization rule with invalid regex {}: {source}",
        python_repr(pattern)
    )]
    InvalidRegex {
        pattern: String,
        #[source]
        source: Box<fancy_regex::Error>,
    },
    #[error(
        "LiteLLM: skipping malformed fallback generalization rule {label} ('fill_missing_for_providers' must be a list of provider strings)."
    )]
    MalformedFillMissing { label: String },
    #[error(
        "LiteLLM: skipping invalid fallback generalization rule {label}: 'litellm_provider' in 'model_info' must be a string."
    )]
    InvalidProvider { label: String },
}
