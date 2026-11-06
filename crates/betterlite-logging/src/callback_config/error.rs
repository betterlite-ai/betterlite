use rust_decimal::Decimal;
use thiserror::Error;

use super::family::Family;
use crate::logging_privacy::CallbackMetadataError;

const SAMPLING_RATE_RULE: &str = "must be a number between 0.0 and 1.0 (inclusive)";

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error(
        "{var} applies to the arize callback only, not {}",
        callback_name.as_deref().map_or_else(|| "None".to_owned(), |text| format!("'{text}'"))
    )]
    SamplingRateCallback {
        var: String,
        callback_name: Option<String>,
    },
    #[error("{var} {SAMPLING_RATE_RULE}, got '{value}'")]
    SamplingRateUnparsable {
        var: String,
        value: String,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("{var} {SAMPLING_RATE_RULE}, got '{value}'")]
    SamplingRateOutOfRange {
        var: String,
        value: String,
        rate: Decimal,
    },
    #[error(
        "Invalid langfuse_environment '{value}': must be lowercase alphanumerics/hyphens/underscores and must not start with 'langfuse'"
    )]
    LangfuseEnvironment {
        value: String,
        #[source]
        source: CallbackMetadataError,
    },
    #[error(
        "langfuse_span_scope applies to the langfuse_otel callback only, not {}",
        callback_name.as_deref().map_or_else(|| "None".to_owned(), |text| format!("'{text}'"))
    )]
    SpanScopeCallback { callback_name: Option<String> },
    #[error("Invalid langfuse_span_scope '{value}': must be one of ['full', 'llm_only']")]
    SpanScopeValue { value: String },
    #[error("Per-team New Relic routing requires the proxy to run with LITELLM_OTEL_V2=true.")]
    NewRelicRequiresOtelV2,
    #[error("Unknown newrelic_region '{region}'. Supported regions: eu, us.")]
    NewRelicRegion { region: String },
    #[error("newrelic_region requires newrelic_api_key; the region rides the team's own key.")]
    NewRelicRegionNeedsKey,
    #[error(
        "{family} is already configured by another callback entry on this team. Remove that entry before setting {var} here."
    )]
    FamilyConflict { family: Family, var: String },
    #[error(
        "langfuse_span_scope is already set to '{stored}' by another callback entry. Every entry shares one scope: remove that entry or send the same value."
    )]
    SpanScopeConflict { stored: String },
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("the callback config is not valid JSON of the expected shape")]
    Json(#[from] serde_json::Error),
    #[error("callback variable {var} must be a string, a number or null")]
    VariableType { var: String },
}

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Rejected(#[from] ValidationError),
}
