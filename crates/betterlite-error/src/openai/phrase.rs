use std::collections::BTreeMap;

use super::{OpenaiExceptionInput, flat_body, openai_context};
use crate::error::details::{ApiDetails, RateLimitDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ExceptionCheckers;
use crate::rate_limit::RateLimitCategory;

// the error-string phrase a branch of _map_openai_exception matches, in its exact precedence order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PhraseFamily {
    RateLimit,
    ContextWindow,
    ModelNotFound,
    TimeoutString,
    ContentPolicy,
    EncryptedContent,
    InvalidRequest,
    InternalServerText,
    RequestTooLarge,
    MissingCredential,
    MistralStreaming,
}

const MISSING_CREDENTIAL: &str = "The api_key client option must be set either by passing api_key \
to the client or by setting the OPENAI_API_KEY environment variable";

// invalid_request_error carries both the content-policy and the generic bad-request phrasing
fn is_content_policy_text(text: &str) -> bool {
    if text.contains("invalid_request_error") && text.contains("content_policy_violation") {
        return true;
    }
    if text.contains("Invalid prompt") && text.contains("violating our usage policy") {
        return true;
    }
    text.to_lowercase()
        .contains("request was rejected as a result of the safety system")
}

// the three error-string predicates between content-policy and request-too-large in precedence order
fn phrase_family_midrange(text: &str) -> Option<PhraseFamily> {
    if text.contains("invalid_encrypted_content") || text.contains("could not be verified") {
        return Some(PhraseFamily::EncryptedContent);
    }
    if text.contains("invalid_request_error") && !text.contains("Incorrect API key provided") {
        return Some(PhraseFamily::InvalidRequest);
    }
    if text.contains("Web server is returning an unknown error")
        || text.contains("The server had an error processing your request.")
    {
        return Some(PhraseFamily::InternalServerText);
    }
    None
}

// _map_openai_exception's error-string predicates, checked in their exact precedence order
pub(super) fn phrase_family(input: &OpenaiExceptionInput) -> Option<PhraseFamily> {
    let text = input.error_str.as_str();
    if ExceptionCheckers::is_error_str_rate_limit(text, input.status_code) {
        return Some(PhraseFamily::RateLimit);
    }
    if ExceptionCheckers::context_window(text) {
        return Some(PhraseFamily::ContextWindow);
    }
    if text.contains("invalid_request_error") && text.contains("model_not_found") {
        return Some(PhraseFamily::ModelNotFound);
    }
    if text.contains("A timeout occurred") {
        return Some(PhraseFamily::TimeoutString);
    }
    if is_content_policy_text(text) {
        return Some(PhraseFamily::ContentPolicy);
    }
    if let Some(family) = phrase_family_midrange(text) {
        return Some(family);
    }
    if text.contains("Request too large") {
        return Some(PhraseFamily::RequestTooLarge);
    }
    if text.contains(MISSING_CREDENTIAL) {
        return Some(PhraseFamily::MissingCredential);
    }
    if text.contains("Mistral API raised a streaming error") {
        return Some(PhraseFamily::MistralStreaming);
    }
    None
}

const ENCRYPTED_CONTENT_SUFFIX: &str =
    "\n\nEncrypted content affinity is required across deployments.";

// each family's own text prefix/suffix, and whether its branch keeps openai.APIError's body fields
const fn phrase_shape(family: PhraseFamily) -> (&'static str, &'static str, bool) {
    match family {
        PhraseFamily::RateLimit | PhraseFamily::RequestTooLarge => ("RateLimitError: ", "", true),
        PhraseFamily::ContextWindow => ("ContextWindowExceededError: ", "", false),
        PhraseFamily::ContentPolicy => ("ContentPolicyViolationError: ", "", true),
        PhraseFamily::EncryptedContent => ("", ENCRYPTED_CONTENT_SUFFIX, true),
        PhraseFamily::InvalidRequest | PhraseFamily::InternalServerText => ("", "", true),
        PhraseFamily::MissingCredential => ("AuthenticationError: ", "", false),
        PhraseFamily::ModelNotFound
        | PhraseFamily::TimeoutString
        | PhraseFamily::MistralStreaming => ("", "", false),
    }
}

// the ErrorClass variant each family raises, over the one context every family builds the same way
fn phrase_variant(family: PhraseFamily, context: ErrorContext) -> ErrorClass {
    match family {
        PhraseFamily::RateLimit | PhraseFamily::RequestTooLarge => {
            ErrorClass::RateLimit(Box::new(RateLimitDetails {
                context,
                category: RateLimitCategory::VendorRateLimit,
                dimension: None,
                headers: BTreeMap::new(),
            }))
        }
        PhraseFamily::ContextWindow => ErrorClass::ContextWindowExceeded(Box::new(context)),
        PhraseFamily::ModelNotFound => ErrorClass::NotFound(Box::new(context)),
        PhraseFamily::TimeoutString => ErrorClass::Timeout(Box::new(TimeoutDetails {
            context,
            original_status: TimeoutStatus(408),
        })),
        PhraseFamily::ContentPolicy => ErrorClass::ContentPolicyViolation(Box::new(context)),
        PhraseFamily::EncryptedContent | PhraseFamily::InvalidRequest => {
            ErrorClass::BadRequest(Box::new(context))
        }
        PhraseFamily::InternalServerText => ErrorClass::InternalServer(Box::new(context)),
        PhraseFamily::MissingCredential => ErrorClass::Authentication(Box::new(context)),
        PhraseFamily::MistralStreaming => ErrorClass::Api(Box::new(ApiDetails {
            context,
            status: 500,
        })),
    }
}

// builds the ErrorClass each phrase family raises, mirroring every branch's own text, body and variant
pub(super) fn phrase_error_class(
    input: &OpenaiExceptionInput,
    provider: &str,
    message: &str,
    family: PhraseFamily,
) -> ErrorClass {
    let (prefix, suffix, keeps_body) = phrase_shape(family);
    let text = format!("{prefix}{provider} - {message}{suffix}");
    let body = if keeps_body {
        flat_body(input)
    } else {
        super::NO_BODY
    };
    let context = openai_context(input, text, body);
    phrase_variant(family, context)
}
