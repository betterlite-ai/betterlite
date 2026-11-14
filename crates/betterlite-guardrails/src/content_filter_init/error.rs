use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContentFilterInitError {
    #[error("Content Filter: guardrail_name is required")]
    MissingName,
    #[error("the guardrail parameter {} has an invalid value", .0)]
    InvalidParam(String),
}
