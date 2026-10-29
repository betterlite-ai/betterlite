use std::collections::BTreeMap;
use std::error::Error as StdError;

use crate::envelope::{WireField, filter_unsafe_headers};
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

pub mod details;
mod tests;

use details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails,
    RateLimitDetails, SchemaValidationDetails, TimeoutDetails,
};

// the fields every openai.*Error-derived LiteLLM exception carries: a message, model/provider identity, retry counters
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ErrorContext {
    pub message: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub num_retries: Option<u32>,
}

impl ErrorContext {
    // type/param differ by family: most openai.*Error subclasses leave them null, a few set a literal string
    fn metadata(&self, status_code: u16, error_type: WireField, param: WireField) -> ErrorMetadata {
        let retry = RetryMetadata {
            max_retries: self.max_retries,
            num_retries: self.num_retries,
        };
        ErrorMetadata {
            status_code,
            model: self.model.clone(),
            provider: self.provider.clone(),
            error_type,
            param,
            code: WireField::Text(status_code.to_string()),
            retry,
            rate_limit_category: None,
            rate_limit_dimension: None,
            allowed_response_headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct RetryMetadata {
    pub max_retries: Option<u32>,
    pub num_retries: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ErrorMetadata {
    pub status_code: u16,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "type")]
    pub error_type: WireField,
    pub param: WireField,
    pub code: WireField,
    pub retry: RetryMetadata,
    pub rate_limit_category: Option<RateLimitCategory>,
    pub rate_limit_dimension: Option<RateLimitDimension>,
    pub allowed_response_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ErrorClass {
    Authentication(Box<ErrorContext>),
    NotFound(Box<ErrorContext>),
    PermissionDenied(Box<ErrorContext>),
    BadRequest(Box<ErrorContext>),
    UnprocessableEntity(Box<ErrorContext>),
    Timeout(Box<TimeoutDetails>),
    RateLimit(Box<RateLimitDetails>),
    BudgetExceeded(Box<BudgetExceededDetails>),
    ContextWindowExceeded(Box<ErrorContext>),
    ContentPolicyViolation(Box<ErrorContext>),
    ServiceUnavailable(Box<ErrorContext>),
    BadGateway(Box<ErrorContext>),
    InternalServer(Box<ErrorContext>),
    Api(Box<ApiDetails>),
    ApiConnection(Box<ErrorContext>),
    ApiResponseValidation(Box<ErrorContext>),
    JsonSchemaValidation(Box<SchemaValidationDetails>),
    UnsupportedParams(Box<ErrorContext>),
    GuardrailRaised(Box<GuardrailDetails>),
    MidStreamFallback(Box<MidStreamFallbackDetails>),
    // a caught exception outside every known family: every wire field is genuinely missing, like a plain Exception
    Unclassified(Box<ErrorContext>),
}

impl ErrorClass {
    #[must_use]
    pub fn metadata(&self) -> ErrorMetadata {
        if let Some((context, status, error_type, param)) = plain_context_status(self) {
            return context.metadata(status, error_type, param);
        }
        details_metadata(self)
    }
}

type PlainStatus<'a> = (&'a ErrorContext, u16, WireField, WireField);

fn plain_context_status(class: &ErrorClass) -> Option<PlainStatus<'_>> {
    let (context, status, error_type, param) = match class {
        ErrorClass::Authentication(context) => (context, 401, WireField::Null, WireField::Null),
        ErrorClass::NotFound(context) => (context, 404, WireField::Null, WireField::Null),
        ErrorClass::PermissionDenied(context) => {
            (context, 403, WireField::Null, WireField::Null)
        }
        ErrorClass::BadRequest(context)
        | ErrorClass::ContextWindowExceeded(context)
        | ErrorClass::ContentPolicyViolation(context) => {
            (context, 400, WireField::Null, WireField::Null)
        }
        ErrorClass::UnprocessableEntity(context) => {
            (context, 422, WireField::Null, WireField::Null)
        }
        ErrorClass::ServiceUnavailable(context) => {
            (context, 503, WireField::Null, WireField::Null)
        }
        ErrorClass::BadGateway(context) => (context, 502, WireField::Null, WireField::Null),
        ErrorClass::InternalServer(context) => (
            context,
            500,
            WireField::Text("internal_server_error".to_owned()),
            WireField::Null,
        ),
        ErrorClass::ApiConnection(context) | ErrorClass::ApiResponseValidation(context) => {
            (context, 500, WireField::Null, WireField::Null)
        }
        // openai.APIError defaults code/param to None as class attributes; type has no such default and stays unset
        ErrorClass::UnsupportedParams(context) => {
            (context, 400, WireField::MISSING, WireField::Null)
        }
        ErrorClass::Unclassified(context) => {
            (context, 500, WireField::MISSING, WireField::MISSING)
        }
        _ => return None,
    };
    Some((context, status, error_type, param))
}

fn details_metadata(class: &ErrorClass) -> ErrorMetadata {
    match class {
        ErrorClass::Timeout(details) => {
            let status = details.original_status.unwrap_or(408);
            details
                .context
                .metadata(status, WireField::Null, WireField::Null)
        }
        ErrorClass::Api(details) => details
            .context
            .metadata(details.status, WireField::Null, WireField::Null),
        ErrorClass::RateLimit(details) => rate_limit_metadata(details),
        ErrorClass::BudgetExceeded(details) => {
            let mut metadata = bare_metadata(
                details.status_code,
                None,
                details.provider.clone(),
                WireField::MISSING,
                WireField::MISSING,
            );
            metadata.rate_limit_category = Some(RateLimitCategory::LitellmRateLimit);
            metadata.rate_limit_dimension = Some(RateLimitDimension::Budget);
            metadata
        }
        ErrorClass::GuardrailRaised(details) => bare_metadata(
            details.status_code,
            None,
            None,
            WireField::MISSING,
            WireField::MISSING,
        ),
        ErrorClass::MidStreamFallback(details) => {
            let status = details.original_status.unwrap_or(503);
            details
                .context
                .metadata(status, WireField::Null, WireField::Null)
        }
        ErrorClass::JsonSchemaValidation(details) => bare_metadata(
            500,
            Some(details.model.clone()),
            Some(details.provider.clone()),
            WireField::Null,
            WireField::Null,
        ),
        _ => unreachable!("plain_context_status already handles every Box<ErrorContext> variant"),
    }
}

fn bare_metadata(
    status_code: u16,
    model: Option<String>,
    provider: Option<String>,
    error_type: WireField,
    param: WireField,
) -> ErrorMetadata {
    let retry = RetryMetadata {
        max_retries: None,
        num_retries: None,
    };
    ErrorMetadata {
        status_code,
        model,
        provider,
        error_type,
        param,
        code: WireField::Text(status_code.to_string()),
        retry,
        rate_limit_category: None,
        rate_limit_dimension: None,
        allowed_response_headers: BTreeMap::new(),
    }
}

fn rate_limit_metadata(details: &RateLimitDetails) -> ErrorMetadata {
    let mut metadata = details.context.metadata(
        429,
        WireField::Text("throttling_error".to_owned()),
        WireField::Null,
    );
    metadata.rate_limit_category = Some(details.category);
    metadata.rate_limit_dimension = details.dimension;
    metadata.allowed_response_headers = filter_unsafe_headers(details.headers.clone());
    metadata
}

// the #[source] cause stays reachable for logging; classify() only ever hands the client a bounded message
#[derive(Debug, thiserror::Error)]
pub enum TaxonomyError {
    #[error("the request to {provider} timed out")]
    Timeout {
        provider: String,
        model: Option<String>,
        #[source]
        cause: Box<dyn StdError + Send + Sync>,
    },
    #[error("the connection to {provider} failed")]
    Transport {
        provider: String,
        model: Option<String>,
        #[source]
        cause: Box<dyn StdError + Send + Sync>,
    },
    #[error("the response from {provider} could not be parsed")]
    Parse {
        provider: String,
        model: Option<String>,
        #[source]
        cause: Box<dyn StdError + Send + Sync>,
    },
}

impl TaxonomyError {
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::Timeout {
                provider, model, ..
            } => ErrorClass::Timeout(Box::new(TimeoutDetails {
                context: ErrorContext {
                    message: self.to_string(),
                    model: model.clone(),
                    provider: Some(provider.clone()),
                    max_retries: None,
                    num_retries: None,
                },
                original_status: None,
            })),
            Self::Transport {
                provider, model, ..
            } => ErrorClass::ApiConnection(Box::new(ErrorContext {
                message: self.to_string(),
                model: model.clone(),
                provider: Some(provider.clone()),
                max_retries: None,
                num_retries: None,
            })),
            Self::Parse {
                provider, model, ..
            } => ErrorClass::ApiResponseValidation(Box::new(ErrorContext {
                message: self.to_string(),
                model: model.clone(),
                provider: Some(provider.clone()),
                max_retries: None,
                num_retries: None,
            })),
        }
    }
}
