use crate::error::{ErrorClass, ErrorContext};

mod tests;

// mirrors _map_cloudflare_exception's two independent `if` checks; neither is an elif of the other
#[must_use]
pub fn map_cloudflare_exception(error_str: &str, model: Option<String>) -> Option<ErrorClass> {
    if error_str.contains("Authentication error") {
        return Some(ErrorClass::Authentication(Box::new(ErrorContext {
            message: "the provider rejected the request as unauthorized".to_owned(),
            model,
            provider: Some("cloudflare".to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        })));
    }
    if error_str.contains("must have required property") {
        return Some(ErrorClass::BadRequest(Box::new(ErrorContext {
            message: "the request body is missing a required property".to_owned(),
            model,
            provider: Some("cloudflare".to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        })));
    }
    None
}
