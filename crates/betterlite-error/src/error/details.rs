use std::collections::BTreeMap;
use std::fmt;

use rust_decimal::Decimal;

use super::ErrorContext;
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

// exception_status_code in Timeout.__init__ defaults to 408 when the caller leaves it unset
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TimeoutStatus(pub u16);

impl Default for TimeoutStatus {
    fn default() -> Self {
        Self(408)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimeoutDetails {
    pub context: ErrorContext,
    #[serde(default)]
    pub original_status: TimeoutStatus,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RateLimitDetails {
    pub context: ErrorContext,
    pub category: RateLimitCategory,
    #[serde(default)]
    pub dimension: Option<RateLimitDimension>,
    // the explicit headers= kwarg; RateLimitError never derives these from a response, so they skip llm_provider- projection
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

// never the derived Debug: a vendor Set-Cookie or secret header value must not escape through format!("{:?}")
impl fmt::Debug for RateLimitDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimitDetails")
            .field("context", &self.context)
            .field("category", &self.category)
            .field("dimension", &self.dimension)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

// response.status_code when openai.APIStatusError.__init__ gets one; DEFAULT is the mapper's own status otherwise
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ResponseStatus<const DEFAULT: u16>(pub u16);

impl<const DEFAULT: u16> Default for ResponseStatus<DEFAULT> {
    fn default() -> Self {
        Self(DEFAULT)
    }
}

// flat, not a nested `context: ErrorContext`, so callers outside this crate keep pattern-matching the same field names
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResponseDetails<const DEFAULT: u16> {
    pub message: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub num_retries: Option<u32>,
    #[serde(default)]
    pub body_type: Option<String>,
    #[serde(default)]
    pub body_param: Option<String>,
    #[serde(default)]
    pub body_code: Option<String>,
    #[serde(default)]
    pub status: ResponseStatus<DEFAULT>,
}

impl<const DEFAULT: u16> From<ErrorContext> for ResponseDetails<DEFAULT> {
    fn from(context: ErrorContext) -> Self {
        Self {
            message: context.message,
            model: context.model,
            provider: context.provider,
            max_retries: context.max_retries,
            num_retries: context.num_retries,
            body_type: context.body_type,
            body_param: context.body_param,
            body_code: context.body_code,
            status: ResponseStatus::default(),
        }
    }
}

impl<const DEFAULT: u16> ResponseDetails<DEFAULT> {
    // the status a mapper carries off the provider's response, falling back to this variant's own class default
    pub fn with_status(context: ErrorContext, status_code: Option<u16>) -> Self {
        Self {
            status: status_code.map(ResponseStatus).unwrap_or_default(),
            ..Self::from(context)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApiDetails {
    pub context: ErrorContext,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BudgetExceededDetails {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub current_cost: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub max_budget: Decimal,
    #[serde(default)]
    pub message: Option<String>,
    // llm_provider or "" in BudgetExceededError.__init__: always a string, never absent
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub entity_id: Option<String>,
    // litellm.budget_exceeded_status_code, 422 by default
    pub status_code: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuardrailDetails {
    #[serde(default)]
    pub guardrail_name: Option<String>,
    pub message: String,
    pub status_code: u16,
    pub blocked_content: bool,
}

// getattr(original_exception, "status_code", None) defaults to 503 when the upstream exception has none
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct MidStreamStatus(pub u16);

impl Default for MidStreamStatus {
    fn default() -> Self {
        Self(503)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MidStreamFallbackDetails {
    pub context: ErrorContext,
    #[serde(default)]
    pub original_status: MidStreamStatus,
    pub generated_content: String,
    pub is_pre_first_chunk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SchemaValidationDetails {
    pub model: String,
    pub provider: String,
    pub raw_response: String,
    pub schema: String,
}
