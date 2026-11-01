use crate::error::{ErrorClass, ErrorContext};

mod tests;

// mirrors the _map_vllm_exception kwargs its single branch predicate reads
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VllmExceptionInput {
    pub model: String,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

#[must_use]
pub fn map_vllm_exception(input: &VllmExceptionInput) -> Option<ErrorClass> {
    if input.status_code != Some(0) {
        return None;
    }
    let message = format!("vllm: {}", input.message.clone().unwrap_or_default());
    Some(ErrorClass::ApiConnection(Box::new(ErrorContext {
        message,
        model: Some(input.model.clone()),
        provider: Some("vllm".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    })))
}
