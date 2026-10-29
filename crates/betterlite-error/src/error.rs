use std::collections::BTreeMap;
use std::io::Error as IoError;

use crate::envelope::WireField;

pub mod details;
pub mod metadata;
mod tests;

use details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails,
    RateLimitDetails, SchemaValidationDetails, TimeoutDetails, TimeoutStatus,
};
use metadata::{ErrorMetadata, RetryMetadata, rate_limit_metadata};

use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

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

// which wire fields a body dict fills, matching which LiteLLM subclass forwards body= to openai.APIError.__init__
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyOverride {
    // most families: openai.APIError never receives a body, so type/param/code stay at their class default
    None,
    // BadRequest, ContentPolicyViolation, Api: openai.APIError reads type/param/code straight off the body
    Full,
    // InternalServer, RateLimit: the subclass overwrites self.type with a literal right after openai.APIError runs
    TypeLiteral,
}

impl ErrorContext {
    fn metadata(
        &self,
        status_code: u16,
        default_type: WireField,
        default_param: WireField,
        body: BodyOverride,
    ) -> ErrorMetadata {
        let retry = RetryMetadata {
            max_retries: self.max_retries,
            num_retries: self.num_retries,
        };
        let (error_type, param, code) = match body {
            BodyOverride::None => (default_type, default_param, None),
            BodyOverride::Full => (
                self.body_type.clone().map_or(default_type, WireField::Text),
                self.body_param
                    .clone()
                    .map_or(default_param, WireField::Text),
                self.body_code.clone(),
            ),
            BodyOverride::TypeLiteral => (
                default_type,
                self.body_param
                    .clone()
                    .map_or(default_param, WireField::Text),
                self.body_code.clone(),
            ),
        };
        ErrorMetadata {
            status_code,
            model: self.model.clone(),
            provider: self.provider.clone(),
            error_type,
            param,
            code: status_code.to_string(),
            retry,
            rate_limit_category: None,
            rate_limit_dimension: None,
            allowed_response_headers: BTreeMap::new(),
            openai_code: code,
        }
    }
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
        if let Self::RateLimit(details) = self {
            return rate_limit_metadata(details);
        }
        if let Self::BudgetExceeded(details) = self {
            return ErrorMetadata {
                status_code: details.status_code,
                model: None,
                provider: Some(details.provider.clone()),
                error_type: WireField::MISSING,
                param: WireField::MISSING,
                code: details.status_code.to_string(),
                retry: RetryMetadata::default(),
                rate_limit_category: Some(RateLimitCategory::LitellmRateLimit),
                rate_limit_dimension: Some(RateLimitDimension::Budget),
                allowed_response_headers: BTreeMap::new(),
                openai_code: None,
            };
        }
        if let Self::GuardrailRaised(details) = self {
            return ErrorMetadata {
                status_code: details.status_code,
                model: None,
                provider: None,
                error_type: WireField::MISSING,
                param: WireField::MISSING,
                code: details.status_code.to_string(),
                retry: RetryMetadata::default(),
                rate_limit_category: None,
                rate_limit_dimension: None,
                allowed_response_headers: BTreeMap::new(),
                openai_code: None,
            };
        }
        self.context_metadata()
    }

    // openai.APIError defaults code/param to None as class attributes; type has no such default and stays unset
    fn context_metadata(&self) -> ErrorMetadata {
        use BodyOverride::{Full, None as No, TypeLiteral};
        use ErrorClass::{
            ApiConnection, ApiResponseValidation, BadRequest, BudgetExceeded,
            ContentPolicyViolation, GuardrailRaised, MidStreamFallback, RateLimit,
        };
        use WireField::Null;
        const MISSING: WireField = WireField::MISSING;
        let ise = WireField::Text("internal_server_error".to_owned());
        match self {
            Self::Authentication(ctx) => ctx.metadata(401, Null, Null, No),
            Self::NotFound(ctx) => ctx.metadata(404, Null, Null, No),
            Self::PermissionDenied(ctx) => ctx.metadata(403, Null, Null, No),
            BadRequest(ctx) | ContentPolicyViolation(ctx) => ctx.metadata(400, Null, Null, Full),
            Self::ContextWindowExceeded(ctx) => ctx.metadata(400, Null, Null, No),
            Self::UnprocessableEntity(ctx) => ctx.metadata(422, Null, Null, No),
            Self::ServiceUnavailable(ctx) => ctx.metadata(503, Null, Null, No),
            Self::BadGateway(ctx) => ctx.metadata(502, Null, Null, No),
            Self::InternalServer(ctx) => ctx.metadata(500, ise, Null, TypeLiteral),
            ApiConnection(ctx) | ApiResponseValidation(ctx) => ctx.metadata(500, Null, Null, No),
            Self::UnsupportedParams(ctx) => ctx.metadata(400, MISSING, Null, No),
            Self::Unclassified(ctx) => ctx.metadata(500, MISSING, MISSING, No),
            Self::Timeout(to) => to.context.metadata(to.original_status.0, Null, Null, No),
            Self::Api(api) => api.context.metadata(api.status, Null, Null, Full),
            MidStreamFallback(fb) => fb.context.metadata(fb.original_status.0, Null, Null, No),
            Self::JsonSchemaValidation(details) => ErrorMetadata {
                status_code: 500,
                model: Some(details.model.clone()),
                provider: Some(details.provider.clone()),
                error_type: Null,
                param: Null,
                code: 500.to_string(),
                retry: RetryMetadata::default(),
                rate_limit_category: None,
                rate_limit_dimension: None,
                allowed_response_headers: BTreeMap::new(),
                openai_code: None,
            },
            RateLimit(_) | BudgetExceeded(_) | GuardrailRaised(_) => self.metadata(),
        }
    }
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
                original_status: TimeoutStatus(408),
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
