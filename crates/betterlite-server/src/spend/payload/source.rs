use betterlite_logging::logging_metadata::decode::decode;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_with::rust::double_option as nullable;

use super::SpendPayloadError;

const STANDARD_RECORD: &str = "standard logging payload";

// The standard logging payload fields get_logging_payload reads; every other field belongs to the payload's own owner.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StandardSource {
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub model_group: Option<String>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    #[serde(deserialize_with = "nullable::deserialize")]
    pub total_tokens: Option<Option<i64>>,
    #[serde(deserialize_with = "nullable::deserialize")]
    pub prompt_tokens: Option<Option<i64>>,
    #[serde(deserialize_with = "nullable::deserialize")]
    pub completion_tokens: Option<Option<i64>>,
    pub request_tags: Option<Json>,
    pub metadata: Option<StandardMetadataSource>,
    pub hidden_params: Option<HiddenParamsSource>,
    pub model_map_information: Option<JsonObject>,
    pub guardrail_information: Option<Json>,
    pub cost_breakdown: Option<CostBreakdown>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub autorouter_savings: Option<Decimal>,
    pub autorouter_savings_estimate: Option<JsonObject>,
    #[serde(rename = "autorouter_baseline_observation")]
    pub autorouter_baseline: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StandardMetadataSource {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_alias: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub applied_guardrails: Option<Vec<String>>,
    pub mcp_tool_call_metadata: Option<JsonObject>,
    pub vector_store_request_metadata: Option<Vec<Json>>,
    pub usage_object: Option<JsonObject>,
    pub cold_storage_object_key: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct HiddenParamsSource {
    #[serde(deserialize_with = "Option::deserialize")]
    pub litellm_overhead_time_ms: Option<Decimal>,
    pub batch_models: Option<Vec<String>>,
    pub batch_successful_requests: Option<i64>,
    pub batch_failed_requests: Option<i64>,
    pub additional_headers: Option<JsonObject>,
}

#[derive(Clone, Copy, Debug)]
pub enum AttributionField {
    ModelId,
    ModelGroup,
    ApiBase,
    CustomLlmProvider,
}

impl AttributionField {
    const fn read(self, source: &StandardSource) -> Option<&String> {
        match self {
            Self::ModelId => source.model_id.as_ref(),
            Self::ModelGroup => source.model_group.as_ref(),
            Self::ApiBase => source.api_base.as_ref(),
            Self::CustomLlmProvider => source.custom_llm_provider.as_ref(),
        }
    }
}

// _sl_attribution_fallback: a None payload and any falsey field both read as the empty string.
#[must_use]
pub fn sl_attribution_fallback(
    standard: Option<&StandardSource>,
    field: AttributionField,
) -> String {
    let Some(source) = standard else {
        return String::new();
    };
    field.read(source).cloned().unwrap_or_default()
}

// kwargs.get("standard_logging_object", None), decoded before any of its fields is read.
pub(crate) fn standard_source(
    value: Option<&Json>,
) -> Result<Option<StandardSource>, SpendPayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => {
            let source = decode(STANDARD_RECORD, fields.clone())?;
            Ok(Some(source))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpendPayloadError::FieldType {
                field: "standard_logging_object",
            })
        }
    }
}
