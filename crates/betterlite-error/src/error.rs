//! Shared provider error taxonomy, ported from `litellm/exceptions.py`.
//!
//! [`ErrorClass`] is a closed classification of every public `LiteLLM` exception family.
//! Each variant carries only the fields the matching Python class sets, so
//! [`ErrorClass::metadata`] can reproduce the exact `getattr(e, field, "None")` three-state
//! wire value (missing attribute, present-and-null, present-and-text) the proxy's catch-all
//! handler reads.

use std::collections::BTreeMap;
use std::error::Error as StdError;

use crate::envelope::{WireField, filter_unsafe_headers};
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

mod tests;

/// The fields common to every `openai.*Error`-derived `LiteLLM` exception: a message, the
/// model/provider identity, and the retry counters threaded through the constructor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ErrorContext {
    pub message: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub max_retries: Option<u32>,
    pub num_retries: Option<u32>,
}

impl ErrorContext {
    #[must_use]
    pub const fn new(message: String, model: Option<String>, provider: Option<String>) -> Self {
        Self {
            message,
            model,
            provider,
            max_retries: None,
            num_retries: None,
        }
    }

    #[must_use]
    pub const fn with_retry(mut self, max_retries: Option<u32>, num_retries: Option<u32>) -> Self {
        self.max_retries = max_retries;
        self.num_retries = num_retries;
        self
    }

    /// `type` and `param` differ by family: most `openai.*Error` subclasses set them to
    /// `None` (present, so the wire value is JSON `null`), a few set a literal string.
    fn metadata(&self, status_code: u16, error_type: WireField, param: WireField) -> ErrorMetadata {
        ErrorMetadata {
            status_code,
            model: self.model.clone(),
            provider: self.provider.clone(),
            error_type,
            param,
            code: WireField::text(status_code.to_string()),
            retry: RetryMetadata {
                max_retries: self.max_retries,
                num_retries: self.num_retries,
            },
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

/// Immutable, oracle-comparable metadata for one constructed [`ErrorClass`] value.
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TimeoutDetails {
    pub context: ErrorContext,
    /// `exception_status_code` in `Timeout.__init__`; `None` means the 408 default applies.
    pub original_status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RateLimitDetails {
    pub context: ErrorContext,
    pub category: RateLimitCategory,
    pub dimension: Option<RateLimitDimension>,
    /// The explicit `headers=` kwarg. Unlike most families, `RateLimitError` never derives
    /// these from a provider response, so they bypass `llm_provider-` projection entirely.
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApiDetails {
    pub context: ErrorContext,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct BudgetExceededDetails {
    pub current_cost: f64,
    pub max_budget: f64,
    pub message: Option<String>,
    pub provider: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    /// `litellm.budget_exceeded_status_code`, 422 by default.
    pub status_code: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GuardrailDetails {
    pub guardrail_name: Option<String>,
    pub message: String,
    pub status_code: u16,
    pub blocked_content: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MidStreamFallbackDetails {
    pub context: ErrorContext,
    /// `getattr(original_exception, "status_code", None)`; `None` means the 503 default applies.
    pub original_status: Option<u16>,
    pub generated_content: String,
    pub is_pre_first_chunk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SchemaValidationDetails {
    pub model: String,
    pub provider: String,
    pub raw_response: String,
    pub schema: String,
}

/// A closed classification of every public `LiteLLM` exception family this crate represents.
/// Every variant is boxed so none of them dominate the enum's size.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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
    /// A caught exception outside every known family: every wire field is genuinely
    /// missing, matching a plain `Exception` the proxy's catch-all handler still wraps.
    Unclassified(Box<ErrorContext>),
}

impl ErrorClass {
    /// One dispatch table over every family this crate represents; it is as long as the
    /// enum it mirrors, so splitting it would only move its lines into an unreachable arm.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "a flat dispatch table over 21 variants, each one line"
    )]
    pub fn metadata(&self) -> ErrorMetadata {
        match self {
            Self::Authentication(context) => {
                context.metadata(401, WireField::Null, WireField::Null)
            }
            Self::NotFound(context) => context.metadata(404, WireField::Null, WireField::Null),
            Self::PermissionDenied(context) => {
                context.metadata(403, WireField::Null, WireField::Null)
            }
            Self::BadRequest(context)
            | Self::ContextWindowExceeded(context)
            | Self::ContentPolicyViolation(context) => {
                context.metadata(400, WireField::Null, WireField::Null)
            }
            Self::UnprocessableEntity(context) => {
                context.metadata(422, WireField::Null, WireField::Null)
            }
            Self::ServiceUnavailable(context) => {
                context.metadata(503, WireField::Null, WireField::Null)
            }
            Self::BadGateway(context) => context.metadata(502, WireField::Null, WireField::Null),
            Self::InternalServer(context) => context.metadata(
                500,
                WireField::text("internal_server_error".to_owned()),
                WireField::Null,
            ),
            Self::ApiConnection(context) | Self::ApiResponseValidation(context) => {
                context.metadata(500, WireField::Null, WireField::Null)
            }
            // UnsupportedParamsError never calls its openai.BadRequestError base constructor,
            // so .type/.param are never set on the instance: a genuinely missing attribute.
            Self::UnsupportedParams(context) => {
                context.metadata(400, WireField::Missing, WireField::Missing)
            }
            Self::Unclassified(context) => {
                context.metadata(500, WireField::Missing, WireField::Missing)
            }
            Self::Timeout(details) => {
                let status = details.original_status.unwrap_or(408);
                details
                    .context
                    .metadata(status, WireField::Null, WireField::Null)
            }
            Self::Api(details) => {
                details
                    .context
                    .metadata(details.status, WireField::Null, WireField::Null)
            }
            Self::RateLimit(details) => rate_limit_metadata(details),
            Self::BudgetExceeded(details) => budget_exceeded_metadata(details),
            Self::GuardrailRaised(details) => guardrail_metadata(details),
            Self::MidStreamFallback(details) => {
                let status = details.original_status.unwrap_or(503);
                details
                    .context
                    .metadata(status, WireField::Null, WireField::Null)
            }
            Self::JsonSchemaValidation(details) => schema_validation_metadata(details),
        }
    }
}

fn rate_limit_metadata(details: &RateLimitDetails) -> ErrorMetadata {
    let mut metadata = details.context.metadata(
        429,
        WireField::text("throttling_error".to_owned()),
        WireField::Null,
    );
    metadata.rate_limit_category = Some(details.category);
    metadata.rate_limit_dimension = details.dimension;
    metadata.allowed_response_headers = filter_unsafe_headers(details.headers.clone());
    metadata
}

fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
    ErrorMetadata {
        status_code: details.status_code,
        model: None,
        provider: details.provider.clone(),
        error_type: WireField::Missing,
        param: WireField::Missing,
        code: WireField::text(details.status_code.to_string()),
        retry: RetryMetadata {
            max_retries: None,
            num_retries: None,
        },
        rate_limit_category: Some(RateLimitCategory::LitellmRateLimit),
        rate_limit_dimension: Some(RateLimitDimension::Budget),
        allowed_response_headers: BTreeMap::new(),
    }
}

fn guardrail_metadata(details: &GuardrailDetails) -> ErrorMetadata {
    ErrorMetadata {
        status_code: details.status_code,
        model: None,
        provider: None,
        error_type: WireField::Missing,
        param: WireField::Missing,
        code: WireField::text(details.status_code.to_string()),
        retry: RetryMetadata {
            max_retries: None,
            num_retries: None,
        },
        rate_limit_category: None,
        rate_limit_dimension: None,
        allowed_response_headers: BTreeMap::new(),
    }
}

fn schema_validation_metadata(details: &SchemaValidationDetails) -> ErrorMetadata {
    ErrorMetadata {
        status_code: 500,
        model: Some(details.model.clone()),
        provider: Some(details.provider.clone()),
        error_type: WireField::Null,
        param: WireField::Null,
        code: WireField::text("500".to_owned()),
        retry: RetryMetadata {
            max_retries: None,
            num_retries: None,
        },
        rate_limit_category: None,
        rate_limit_dimension: None,
        allowed_response_headers: BTreeMap::new(),
    }
}

/// Transport-level failures a provider adapter wraps before they reach [`ErrorClass`].
///
/// The `#[source]` cause stays reachable to internal logging; [`TaxonomyError::classify`]
/// only ever hands the client a bounded, betterlite-owned message.
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
                context: ErrorContext::new(self.to_string(), model.clone(), Some(provider.clone())),
                original_status: None,
            })),
            Self::Transport {
                provider, model, ..
            } => ErrorClass::ApiConnection(Box::new(ErrorContext::new(
                self.to_string(),
                model.clone(),
                Some(provider.clone()),
            ))),
            Self::Parse {
                provider, model, ..
            } => ErrorClass::ApiResponseValidation(Box::new(ErrorContext::new(
                self.to_string(),
                model.clone(),
                Some(provider.clone()),
            ))),
        }
    }
}
