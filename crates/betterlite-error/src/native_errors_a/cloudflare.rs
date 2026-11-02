use crate::error::details::ResponseDetails;
use crate::error::{ErrorClass, ErrorContext};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Authentication,
    BadRequest,
}

// mirrors _map_cloudflare_exception's two independent `if` checks; neither is an elif of the other
fn classify(error_str: &str) -> Option<(Family, &'static str)> {
    if error_str.contains("Authentication error") {
        return Some((
            Family::Authentication,
            "the provider rejected the request as unauthorized",
        ));
    }
    if error_str.contains("must have required property") {
        return Some((
            Family::BadRequest,
            "the request body is missing a required property",
        ));
    }
    None
}

// both _map_cloudflare_exception branches pass response=getattr(original_exception, "response", None)
fn classified_error(
    family: Family,
    reason: &str,
    model: Option<String>,
    status_code: Option<u16>,
) -> ErrorClass {
    let context = ErrorContext {
        message: reason.to_owned(),
        model,
        provider: Some("cloudflare".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    match family {
        Family::Authentication => {
            ErrorClass::Authentication(Box::new(ResponseDetails::with_status(context, status_code)))
        }
        Family::BadRequest => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(context, status_code)))
        }
    }
}

// native.rs's dispatch table calls this two-argument form; it keeps the class default status since that call site sits outside this task's scope
#[must_use]
pub fn map_cloudflare_exception(error_str: &str, model: Option<String>) -> Option<ErrorClass> {
    let (family, reason) = classify(error_str)?;
    Some(classified_error(family, reason, model, None))
}

#[must_use]
pub fn map_cloudflare_with_status(
    error_str: &str,
    model: Option<String>,
    status_code: Option<u16>,
) -> Option<ErrorClass> {
    let (family, reason) = classify(error_str)?;
    Some(classified_error(family, reason, model, status_code))
}
