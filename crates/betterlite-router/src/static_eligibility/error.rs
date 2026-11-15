use crate::static_eligibility::environment::EnvironmentError;
use crate::static_eligibility::tokens::ContextWindowExceeded;
use crate::static_eligibility::web_search::WebSearchError;

// Everything `_pre_call_checks` and the environment gate raise; none of them is a 500.
#[derive(Debug, thiserror::Error)]
pub enum EligibilityError {
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    WebSearch(#[from] WebSearchError),
    #[error(transparent)]
    ContextWindow(#[from] ContextWindowExceeded),
    #[error("Model '{model}' is currently paused and cannot accept requests.")]
    Blocked { model: String },
}
