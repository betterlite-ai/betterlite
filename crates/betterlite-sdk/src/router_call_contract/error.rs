use betterlite_core::api::passthrough_call::PassthroughCallError;
use betterlite_error::error::{ErrorClass, class_message};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GenericError {
    #[error("no executor is registered for the call type {0}")]
    UnknownCallType(String),
    #[error(transparent)]
    Failed(Box<GenericFailure>),
}

// The exception LiteLLM stamps with the two deployment ids its retry and cooldown checks read.
#[derive(Debug, Error)]
#[error("{cause}")]
pub struct GenericFailure {
    #[source]
    pub cause: FailureCause,
    pub failed_deployment_id: Option<String>,
    pub retry_skip_deployment_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum FailureCause {
    #[error("{}", class_message(.0))]
    Class(ErrorClass),
    #[error(transparent)]
    Passthrough(PassthroughCallError),
    #[error("the {key} argument is not {expected}")]
    Argument { key: String, expected: &'static str },
}

impl From<ErrorClass> for GenericError {
    fn from(class: ErrorClass) -> Self {
        Self::Failed(Box::new(GenericFailure {
            cause: FailureCause::Class(class),
            failed_deployment_id: None,
            retry_skip_deployment_id: None,
        }))
    }
}

impl From<PassthroughCallError> for GenericError {
    fn from(error: PassthroughCallError) -> Self {
        Self::Failed(Box::new(GenericFailure {
            cause: FailureCause::Passthrough(error),
            failed_deployment_id: None,
            retry_skip_deployment_id: None,
        }))
    }
}
