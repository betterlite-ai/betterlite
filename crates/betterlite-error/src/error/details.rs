use std::collections::BTreeMap;

use super::ErrorContext;
use crate::rate_limit::{RateLimitCategory, RateLimitDimension};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimeoutDetails {
    pub context: ErrorContext,
    // exception_status_code in Timeout.__init__; None means the 408 default applies
    #[serde(default)]
    pub original_status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RateLimitDetails {
    pub context: ErrorContext,
    pub category: RateLimitCategory,
    #[serde(default)]
    pub dimension: Option<RateLimitDimension>,
    // the explicit headers= kwarg; RateLimitError never derives these from a response, so they skip llm_provider- projection
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApiDetails {
    pub context: ErrorContext,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BudgetExceededDetails {
    pub current_cost: f64,
    pub max_budget: f64,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MidStreamFallbackDetails {
    pub context: ErrorContext,
    // getattr(original_exception, "status_code", None); None means the 503 default applies
    #[serde(default)]
    pub original_status: Option<u16>,
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
