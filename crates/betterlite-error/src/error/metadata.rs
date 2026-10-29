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

// the three families with no ErrorContext to build from, kept as one match so their fields never read as duplicate functions
pub(super) enum Family<'details> {
    Budget(&'details BudgetExceededDetails),
    Guardrail(&'details GuardrailDetails),
    Schema(&'details SchemaValidationDetails),
}

pub(super) fn family_metadata(input: &Family<'_>) -> ErrorMetadata {
    let (status_code, model, provider, error_type, rate_limit_category, rate_limit_dimension) =
        match input {
            Family::Budget(budget) => (
                budget.status_code,
                None,
                Some(budget.provider.clone()),
                WireField::MISSING,
                Some(RateLimitCategory::LitellmRateLimit),
                Some(RateLimitDimension::Budget),
            ),
            Family::Guardrail(guardrail) => (
                guardrail.status_code,
                None,
                None,
                WireField::MISSING,
                None,
                None,
            ),
            Family::Schema(schema) => (
                500,
                Some(schema.model.clone()),
                Some(schema.provider.clone()),
                WireField::Null,
                None,
                None,
            ),
        };
    ErrorMetadata {
        status_code,
        model,
        provider,
        error_type: error_type.clone(),
        param: error_type,
        code: status_code.to_string(),
        retry: RetryMetadata::default(),
        rate_limit_category,
        rate_limit_dimension,
        allowed_response_headers: BTreeMap::new(),
        openai_code: None,
    }
}
