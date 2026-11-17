use thiserror::Error;

// LiteLLM raises inside the hook's try block; the same failures arrive here as its logged message text.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum VectorStoresError {
    #[error("the {field} field must be a list of retrieval records")]
    FieldNotList { field: &'static str },
    #[error("the litellm_logging_obj field must be an object")]
    LoggingNotObject,
    #[error("the litellm_logging_obj.model_call_details field must be an object")]
    CallDetailsNotObject,
    #[error("choice {index} has a {member} that is not an object")]
    TargetNotObject { index: usize, member: &'static str },
    #[error("choice {index} has provider_specific_fields that are not an object")]
    ProviderFieldsNotObject { index: usize },
}
