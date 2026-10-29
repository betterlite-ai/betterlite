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
    pub code: String,
    pub retry: RetryMetadata,
    pub rate_limit_category: Option<RateLimitCategory>,
    pub rate_limit_dimension: Option<RateLimitDimension>,
    pub allowed_response_headers: BTreeMap<String, String>,
    // getattr(e, "code", None) in proxy_server.py; distinct from `code`, which always stringifies the status
    pub openai_code: Option<String>,
}

// the inputs that genuinely differ across the "no retry, no body override" error families; simple_metadata owns the rest
pub(super) struct MetadataFields {
    pub status_code: u16,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub error_type: WireField,
    pub param: WireField,
    pub rate_limit_category: Option<RateLimitCategory>,
    pub rate_limit_dimension: Option<RateLimitDimension>,
}

pub(super) fn simple_metadata(fields: MetadataFields) -> ErrorMetadata {
    let status_code = fields.status_code;
    ErrorMetadata {
        status_code,
        model: fields.model,
        provider: fields.provider,
        error_type: fields.error_type,
        param: fields.param,
        code: status_code.to_string(),
        retry: RetryMetadata::default(),
        rate_limit_category: fields.rate_limit_category,
        rate_limit_dimension: fields.rate_limit_dimension,
        allowed_response_headers: BTreeMap::new(),
        openai_code: None,
    }
}

pub(super) fn budget_exceeded_metadata(details: &BudgetExceededDetails) -> ErrorMetadata {
    let status_code = details.status_code;
    simple_metadata(MetadataFields {
        status_code,
        model: None,
        provider: Some(details.provider.clone()),
        error_type: WireField::MISSING,
        param: WireField::MISSING,
        rate_limit_category: Some(RateLimitCategory::LitellmRateLimit),
        rate_limit_dimension: Some(RateLimitDimension::Budget),
    })
}

pub(super) fn schema_validation_metadata(details: &SchemaValidationDetails) -> ErrorMetadata {
    let model = details.model.clone();
    simple_metadata(MetadataFields {
        status_code: 500,
        model: Some(model),
        provider: Some(details.provider.clone()),
        error_type: WireField::Null,
        param: WireField::Null,
        rate_limit_category: None,
        rate_limit_dimension: None,
    })
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
