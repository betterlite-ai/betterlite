use std::collections::BTreeMap;
use std::fmt;

use super::BodyOverride;
use super::details::{
    BudgetExceededDetails, GuardrailDetails, RateLimitDetails, SchemaValidationDetails,
};
use crate::envelope::{WireField, is_unsafe_header_name};
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RetryMetadata {
    pub max_retries: Option<u32>,
    pub num_retries: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct ErrorMetadata {
    pub status_code: u16,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "type")]
    pub error_type: WireField,
    pub param: WireField,
    pub code: String,
    pub retry: RetryMetadata,
    pub rate_limit_category: Option<RateLimitCategory>,
    pub rate_limit_dimension: Option<RateLimitDimension>,
    pub allowed_response_headers: BTreeMap<String, String>,
    // getattr(e, "code", None) in proxy_server.py; distinct from `code`, which always stringifies the status
    pub openai_code: Option<String>,
}

// never the derived Debug: a header value copied from RateLimitDetails.headers must not escape through format!("{:?}")
impl fmt::Debug for ErrorMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorMetadata")
            .field("status_code", &self.status_code)
            .field("model", &self.model)
            .field("provider", &self.provider)
            .field("error_type", &self.error_type)
            .field("param", &self.param)
            .field("code", &self.code)
            .field("retry", &self.retry)
            .field("rate_limit_category", &self.rate_limit_category)
            .field("rate_limit_dimension", &self.rate_limit_dimension)
            .field(
                "allowed_response_headers",
                &self.allowed_response_headers.keys().collect::<Vec<_>>(),
            )
            .field("openai_code", &self.openai_code)
            .finish()
    }
}

pub(super) fn rate_limit_metadata(details: &RateLimitDetails) -> ErrorMetadata {
    let mut metadata = details.context.metadata(
        429,
        WireField::Text("throttling_error".to_owned()),
        WireField::Null,
        BodyOverride::TypeLiteral,
    );
    // litellm.RateLimitError.__init__ sets self.code = "429" as a literal, after the base openai.APIError body handling
    metadata.openai_code = Some("429".to_owned());
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

pub(super) fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
    ErrorMetadata {
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
    }
}

pub(super) fn guardrail_raised_metadata(details: &GuardrailDetails) -> ErrorMetadata {
    ErrorMetadata {
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
    }
}

pub(super) fn json_schema_validation_metadata(details: &SchemaValidationDetails) -> ErrorMetadata {
    ErrorMetadata {
        status_code: 500,
        model: Some(details.model.clone()),
        provider: Some(details.provider.clone()),
        error_type: WireField::Null,
        param: WireField::Null,
        code: 500.to_string(),
        retry: RetryMetadata::default(),
        rate_limit_category: None,
        rate_limit_dimension: None,
        allowed_response_headers: BTreeMap::new(),
        openai_code: None,
    }
}
