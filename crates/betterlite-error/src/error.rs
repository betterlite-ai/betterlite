use std::collections::BTreeMap;
use std::io::Error as IoError;

use crate::envelope::{WireField, is_unsafe_header_name};
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
    // openai.APIError.__init__ reads these off a body dict; only the LiteLLM subclasses with a body= kwarg ever set them
    #[serde(default)]
    pub body_type: Option<String>,
    #[serde(default)]
    pub body_param: Option<String>,
    #[serde(default)]
    pub body_code: Option<String>,
}

impl ErrorContext {
    // type/param differ by family: most openai.*Error subclasses leave them null, a few set a literal string;
    // a body dict overrides either with its own value, the same way openai.APIError.__init__ does
    fn metadata(
        &self,
        status_code: u16,
        default_type: WireField,
        default_param: WireField,
    ) -> ErrorMetadata {
        let retry = RetryMetadata {
            max_retries: self.max_retries,
            num_retries: self.num_retries,
        };
        let error_type = self.body_type.clone().map_or(default_type, WireField::Text);
        let param = self
            .body_param
            .clone()
            .map_or(default_param, WireField::Text);
        let mut metadata = build_metadata(
            status_code,
            (self.model.clone(), self.provider.clone()),
            (error_type, param),
            retry,
        );
        metadata.openai_code.clone_from(&self.body_code);
        metadata
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
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
    // getattr(e, "code", None) in proxy_server.py; distinct from `code`, which always stringifies the status
    pub openai_code: Option<String>,
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

// the common case: a status code with both wire fields null, before any per-context body override
const fn null_wire(ctx: &ErrorContext, status: u16) -> (&ErrorContext, u16, WireField, WireField) {
    (ctx, status, WireField::Null, WireField::Null)
}

impl ErrorClass {
    #[must_use]
    pub fn metadata(&self) -> ErrorMetadata {
        let (ctx, status, error_type, param) = match self {
            Self::Authentication(ctx) => null_wire(ctx, 401),
            Self::NotFound(ctx) => null_wire(ctx, 404),
            Self::PermissionDenied(ctx) => null_wire(ctx, 403),
            Self::BadRequest(ctx)
            | Self::ContextWindowExceeded(ctx)
            | Self::ContentPolicyViolation(ctx) => null_wire(ctx, 400),
            Self::UnprocessableEntity(ctx) => null_wire(ctx, 422),
            Self::ServiceUnavailable(ctx) => null_wire(ctx, 503),
            Self::BadGateway(ctx) => null_wire(ctx, 502),
            Self::InternalServer(ctx) => (
                ctx.as_ref(),
                500,
                WireField::Text("internal_server_error".to_owned()),
                WireField::Null,
            ),
            Self::ApiConnection(ctx) | Self::ApiResponseValidation(ctx) => null_wire(ctx, 500),
            // openai.APIError defaults code/param to None as class attributes; type has no such default and stays unset
            Self::UnsupportedParams(ctx) => {
                (ctx.as_ref(), 400, WireField::MISSING, WireField::Null)
            }
            Self::Unclassified(ctx) => (ctx.as_ref(), 500, WireField::MISSING, WireField::MISSING),
            Self::Timeout(details) => null_wire(&details.context, details.original_status),
            Self::Api(api) => null_wire(&api.context, api.status),
            Self::MidStreamFallback(details) => {
                null_wire(&details.context, details.original_status)
            }
            Self::RateLimit(details) => return rate_limit_metadata(details),
            Self::BudgetExceeded(details) => return budget_exceeded_metadata(details),
            Self::GuardrailRaised(details) => {
                let wire = (WireField::MISSING, WireField::MISSING);
                return build_metadata(
                    details.status_code,
                    (None, None),
                    wire,
                    RetryMetadata::default(),
                );
            }
            Self::JsonSchemaValidation(details) => return schema_validation_metadata(details),
        };
        ctx.metadata(status, error_type, param)
    }
}

fn schema_validation_metadata(details: &SchemaValidationDetails) -> ErrorMetadata {
    let identity = (Some(details.model.clone()), Some(details.provider.clone()));
    build_metadata(
        500,
        identity,
        (WireField::Null, WireField::Null),
        RetryMetadata::default(),
    )
}

fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
    let mut metadata = build_metadata(
        details.status_code,
        (None, Some(details.provider.clone())),
        (WireField::MISSING, WireField::MISSING),
        RetryMetadata::default(),
    );
    metadata.rate_limit_category = Some(RateLimitCategory::LitellmRateLimit);
    metadata.rate_limit_dimension = Some(RateLimitDimension::Budget);
    metadata
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
        openai_code: None,
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
    metadata.allowed_response_headers = details
        .headers
        .clone()
        .into_iter()
        .filter(|(name, _)| !is_unsafe_header_name(name))
        .collect();
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
                original_status: 408,
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
            body_type: None,
            body_param: None,
            body_code: None,
        }
    }
}
