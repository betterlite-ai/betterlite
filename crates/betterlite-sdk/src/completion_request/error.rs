use super::ports::PromptManagementError;
use crate::completion_options::error::CompletionOptionsError;
use betterlite_context::non_default_params::error::NonDefaultParamsError;
use betterlite_core::message_utilities::validators::ValidationError;
use betterlite_models::headers::affinity::AffinityError;
use betterlite_models::params::ParameterError;
use betterlite_models::request_pricing::error::RequestPricingError;
use betterlite_models::resolution::ResolutionError;
use betterlite_prompts::builtin::PromptError;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::file_ids::limits::FileIdsError;
use betterlite_prompts::messages::MessageNormalizationError;
use betterlite_router::routing_timeouts::error::TimeoutError;
use std::convert::Infallible;
use thiserror::Error;
pub type PricingFailure = RequestPricingError<Infallible>;
pub type RequestResult<T> = Result<T, CompletionRequestError>;
#[derive(Debug, Error)]
pub enum CompletionRequestError {
    #[error("model param not passed in.")]
    ModelNotPassed,
    #[error("the {field} value must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {field} value is not valid for its type")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the request could not be read: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error(transparent)]
    Messages(#[from] MessageNormalizationError),
    #[error(transparent)]
    Control(#[from] CompletionOptionsError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Parameters(#[from] ParameterError),
    #[error(transparent)]
    NonDefault(#[from] NonDefaultParamsError),
    #[error(transparent)]
    PromptHook(#[from] PromptManagementError),
    #[error(transparent)]
    FileIds(#[from] FileIdsError),
    #[error(transparent)]
    SystemMessages(#[from] PromptError),
    #[error(transparent)]
    FunctionPrompt(#[from] FactoryError),
    #[error(transparent)]
    Affinity(#[from] AffinityError),
    #[error(transparent)]
    TimeoutArgument(#[from] TimeoutError),
    #[error(transparent)]
    Pricing(#[from] PricingFailure),
    #[error("the model list entry has no {field} field")]
    ModelListEntry { field: &'static str },
    #[error("This is a mock timeout error")]
    MockTimeout,
}
