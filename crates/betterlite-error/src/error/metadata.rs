use std::collections::BTreeMap;

use super::BodyOverride;
use super::details::{BudgetExceededDetails, RateLimitDetails, SchemaValidationDetails};
use crate::envelope::{WireField, is_unsafe_header_name};
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

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

pub(super) fn schema_validation_metadata(details: &SchemaValidationDetails) -> ErrorMetadata {
    let identity = (Some(details.model.clone()), Some(details.provider.clone()));
    build_metadata(
        500,
        identity,
        (WireField::Null, WireField::Null),
        RetryMetadata::default(),
    )
}

pub(super) fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
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

pub(super) fn build_metadata(
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
