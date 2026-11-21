use std::time::TryFromFloatSecsError;

use betterlite_error::error::{ErrorClass, class_message};
use thiserror::Error;

use crate::router_core::error::RoutingError;
use crate::routing_timeouts::error::TimeoutError;

#[derive(Debug, Error)]
pub enum RetryError {
    // the latest provider error, which carries the retry counters only once the retries ran out
    #[error("{}", class_message(.0))]
    Provider(Box<ErrorClass>),
    #[error(transparent)]
    Routing(#[from] Box<RoutingError>),
    #[error(transparent)]
    Timeout(#[from] TimeoutError),
    #[error("the retry backoff of {seconds} seconds is not a duration")]
    Backoff {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

// async_function_with_retries stamps max_retries and num_retries on the exact types in LITELLM_EXCEPTION_TYPES
pub(super) fn exhausted(mut error: ErrorClass, retries: i64) -> RetryError {
    let count = u32::try_from(retries).ok();
    if let Some((max_retries, num_retries)) = retry_fields(&mut error) {
        *max_retries = count;
        *num_retries = count;
    }
    RetryError::Provider(Box::new(error))
}

type RetryFields<'error> = (&'error mut Option<u32>, &'error mut Option<u32>);

fn retry_fields(error: &mut ErrorClass) -> Option<RetryFields<'_>> {
    let context = match error {
        ErrorClass::Authentication(details) => {
            return Some((&mut details.max_retries, &mut details.num_retries));
        }
        ErrorClass::NotFound(details) => {
            return Some((&mut details.max_retries, &mut details.num_retries));
        }
        ErrorClass::PermissionDenied(details) => {
            return Some((&mut details.max_retries, &mut details.num_retries));
        }
        ErrorClass::UnprocessableEntity(details) => {
            return Some((&mut details.max_retries, &mut details.num_retries));
        }
        ErrorClass::BadRequest(details)
        | ErrorClass::ContextWindowExceeded(details)
        | ErrorClass::ContentPolicyViolation(details) => {
            return Some((&mut details.max_retries, &mut details.num_retries));
        }
        ErrorClass::Timeout(details) => &mut details.context,
        ErrorClass::RateLimit(details) => &mut details.context,
        ErrorClass::Api(details) => &mut details.context,
        ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::UnsupportedParams(context) => &mut **context,
        // JSONSchemaValidationError is listed but keeps no counters; the rest are not LiteLLM exception types
        ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::BudgetExceeded(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => return None,
    };
    Some((&mut context.max_retries, &mut context.num_retries))
}
