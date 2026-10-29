use std::io::Error as IoError;

use crate::envelope::WireField;

pub mod details;
pub mod metadata;
mod tests;

use details::{
    ApiDetails, BudgetExceededDetails, GuardrailDetails, MidStreamFallbackDetails,
    RateLimitDetails, SchemaValidationDetails, TimeoutDetails, TimeoutStatus,
};
use metadata::{
    ErrorMetadata, RetryMetadata, budget_exceeded_metadata, build_metadata, rate_limit_metadata,
    schema_validation_metadata,
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

type MetadataInputs<'ctx> = (&'ctx ErrorContext, u16, WireField, WireField, BodyOverride);

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
        let mut metadata = build_metadata(
            status_code,
            (self.model.clone(), self.provider.clone()),
            (error_type, param),
            retry,
        );
        metadata.openai_code = code;
        metadata
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
        use BodyOverride::{Full, None as No, TypeLiteral};
        use WireField::Null;
        const MISSING: WireField = WireField::MISSING;
        let internal_server_error = WireField::Text("internal_server_error".to_owned());

        // openai.APIError defaults code/param to None as class attributes; type has no such default and stays unset
        let (ctx, status, default_type, default_param, body): MetadataInputs<'_> = match self {
            Self::Authentication(ctx) => (ctx, 401, Null, Null, No),
            Self::NotFound(ctx) => (ctx, 404, Null, Null, No),
            Self::PermissionDenied(ctx) => (ctx, 403, Null, Null, No),
            Self::BadRequest(ctx) | Self::ContentPolicyViolation(ctx) => {
                (ctx, 400, Null, Null, Full)
            }
            Self::ContextWindowExceeded(ctx) => (ctx, 400, Null, Null, No),
            Self::UnprocessableEntity(ctx) => (ctx, 422, Null, Null, No),
            Self::ServiceUnavailable(ctx) => (ctx, 503, Null, Null, No),
            Self::BadGateway(ctx) => (ctx, 502, Null, Null, No),
            Self::InternalServer(ctx) => (ctx, 500, internal_server_error, Null, TypeLiteral),
            Self::ApiConnection(ctx) | Self::ApiResponseValidation(ctx) => {
                (ctx, 500, Null, Null, No)
            }
            Self::UnsupportedParams(ctx) => (ctx, 400, MISSING, Null, No),
            Self::Unclassified(ctx) => (ctx, 500, MISSING, MISSING, No),
            Self::Timeout(timeout) => (&timeout.context, timeout.original_status.0, Null, Null, No),
            Self::Api(api) => (&api.context, api.status, Null, Null, Full),
            Self::MidStreamFallback(mid) => (&mid.context, mid.original_status.0, Null, Null, No),
            Self::RateLimit(details) => return rate_limit_metadata(details),
            Self::BudgetExceeded(details) => return budget_exceeded_metadata(details),
            Self::GuardrailRaised(details) => {
                return build_metadata(
                    details.status_code,
                    (None, None),
                    (MISSING, MISSING),
                    RetryMetadata::default(),
                );
            }
            Self::JsonSchemaValidation(details) => return schema_validation_metadata(details),
        };
        ctx.metadata(status, default_type, default_param, body)
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
