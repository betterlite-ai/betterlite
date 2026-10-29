use std::collections::BTreeMap;
use std::io::Error as IoError;

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
        build_metadata(
            status_code,
            (self.model.clone(), self.provider.clone()),
            (error_type, param),
            retry,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RetryMetadata {
    pub max_retries: Option<u32>,
    pub num_retries: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
        match self {
            Self::Authentication(ctx) => ctx.metadata(401, WireField::Null, WireField::Null),
            Self::NotFound(ctx) => ctx.metadata(404, WireField::Null, WireField::Null),
            Self::PermissionDenied(ctx) => ctx.metadata(403, WireField::Null, WireField::Null),
            Self::BadRequest(ctx)
            | Self::ContextWindowExceeded(ctx)
            | Self::ContentPolicyViolation(ctx) => {
                ctx.metadata(400, WireField::Null, WireField::Null)
            }
            Self::UnprocessableEntity(ctx) => ctx.metadata(422, WireField::Null, WireField::Null),
            Self::ServiceUnavailable(ctx) => ctx.metadata(503, WireField::Null, WireField::Null),
            Self::BadGateway(ctx) => ctx.metadata(502, WireField::Null, WireField::Null),
            Self::InternalServer(ctx) => {
                let error_type = WireField::Text("internal_server_error".to_owned());
                ctx.metadata(500, error_type, WireField::Null)
            }
            Self::ApiConnection(ctx) | Self::ApiResponseValidation(ctx) => {
                ctx.metadata(500, WireField::Null, WireField::Null)
            }
            // openai.APIError defaults code/param to None as class attributes; type has no such default and stays unset
            Self::UnsupportedParams(ctx) => ctx.metadata(400, WireField::MISSING, WireField::Null),
            Self::Unclassified(ctx) => ctx.metadata(500, WireField::MISSING, WireField::MISSING),
            Self::Timeout(details) => timeout_metadata(details),
            Self::Api(details) => {
                let ctx = &details.context;
                ctx.metadata(details.status, WireField::Null, WireField::Null)
            }
            Self::RateLimit(details) => rate_limit_metadata(details),
            Self::BudgetExceeded(details) => budget_exceeded_metadata(details),
            Self::GuardrailRaised(details) => ErrorMetadata {
                status_code: details.status_code,
                code: WireField::Text(details.status_code.to_string()),
                ..ErrorMetadata::default()
            },
            Self::MidStreamFallback(details) => mid_stream_fallback_metadata(details),
            Self::JsonSchemaValidation(details) => {
                let identity = (Some(details.model.clone()), Some(details.provider.clone()));
                let wire = (WireField::Null, WireField::Null);
                build_metadata(500, identity, wire, RetryMetadata::default())
            }
        }
    }
}

fn timeout_metadata(details: &TimeoutDetails) -> ErrorMetadata {
    let status = details.original_status.unwrap_or(408);
    details
        .context
        .metadata(status, WireField::Null, WireField::Null)
}

fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
    let mut metadata = build_metadata(
        details.status_code,
        (None, details.provider.clone()),
        (WireField::MISSING, WireField::MISSING),
        RetryMetadata::default(),
    );
    metadata.rate_limit_category = Some(RateLimitCategory::LitellmRateLimit);
    metadata.rate_limit_dimension = Some(RateLimitDimension::Budget);
    metadata
}

fn mid_stream_fallback_metadata(details: &MidStreamFallbackDetails) -> ErrorMetadata {
    let status = details.original_status.unwrap_or(503);
    details
        .context
        .metadata(status, WireField::Null, WireField::Null)
}

fn build_metadata(
    status_code: u16,
    identity: (Option<String>, Option<String>),
    wire: (WireField, WireField),
    retry: RetryMetadata,
) -> ErrorMetadata {
    let (model, provider) = identity;
    let (error_type, param) = wire;
    let code = WireField::Text(status_code.to_string());
    ErrorMetadata {
        status_code,
        model,
        provider,
        error_type,
        param,
        code,
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
        cause: IoError,
    },
    #[error("the connection to {provider} failed")]
    Transport {
        provider: String,
        model: Option<String>,
        #[source]
        cause: IoError,
    },
    #[error("the response from {provider} could not be parsed")]
    Parse {
        provider: String,
        model: Option<String>,
        #[source]
        cause: serde_json::Error,
    },
}

impl TaxonomyError {
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::Timeout {
                provider, model, ..
            } => ErrorClass::Timeout(Box::new(TimeoutDetails {
                context: self.context(provider, model.as_deref()),
                original_status: None,
            })),
            Self::Transport {
                provider, model, ..
            } => ErrorClass::ApiConnection(Box::new(self.context(provider, model.as_deref()))),
            Self::Parse {
                provider, model, ..
            } => ErrorClass::ApiResponseValidation(Box::new(
                self.context(provider, model.as_deref()),
            )),
        }
    }

    fn context(&self, provider: &str, model: Option<&str>) -> ErrorContext {
        let message = self.to_string();
        ErrorContext {
            message,
            model: model.map(str::to_owned),
            provider: Some(provider.to_owned()),
            max_retries: None,
            num_retries: None,
        }
    }
}
