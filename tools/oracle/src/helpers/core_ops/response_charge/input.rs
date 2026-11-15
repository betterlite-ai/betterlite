use std::collections::BTreeMap;

use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub(super) mod ocr;
pub(super) mod realtime;
pub(super) mod values;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    CompletionCost,
    ResponseCostCalculator,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Chat,
    Image,
    Transcription,
    Ocr,
    RealtimeStream,
    None,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) litellm_provider: String,
    pub(super) mode: String,
    pub(super) input_cost_per_token: Number,
    pub(super) output_cost_per_token: Number,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_read_input_token_cost: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_image: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_query: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ocr_cost_per_credit: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ocr_cost_per_page: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_second: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_token_priority: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_token_priority: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_token_flex: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_token_flex: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) search_context_cost_per_query: Option<BTreeMap<String, Number>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UsageInput {
    #[serde(rename = "prompt_tokens")]
    pub(super) prompt: i64,
    #[serde(rename = "completion_tokens")]
    pub(super) completion: i64,
    #[serde(rename = "cached_tokens")]
    pub(super) cached: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rates {
    pub(super) input_cost_per_token: Decimal,
    pub(super) output_cost_per_token: Decimal,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Margin {
    Percent(Decimal),
    Split {
        percentage: Option<Decimal>,
        fixed_amount: Option<Decimal>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OcrInput {
    pub(super) credits: Option<Number>,
    pub(super) pages_processed: Option<i64>,
    pub(super) pages_processed_annotation: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentInput {
    pub(super) response_cost: Option<Number>,
    pub(super) cost_per_query: Option<Number>,
}

// Only the keys the fixtures send; an unknown built-in tool param is a case-authoring error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StandardTools {
    pub(super) web_search_options: Option<BTreeMap<String, Json>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) function: Function,
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) catalog: BTreeMap<String, Entry>,
    pub(super) usage: UsageInput,
    pub(super) custom_price: Option<Rates>,
    pub(super) cache_hit: Option<bool>,
    pub(super) header_cost: Option<Number>,
    pub(super) discount: BTreeMap<String, Decimal>,
    pub(super) margin: BTreeMap<String, Margin>,
    pub(super) call_type: Option<CallType>,
    pub(super) response_kind: Option<Kind>,
    pub(super) response_model: Option<String>,
    pub(super) optional_params: Option<IndexMap<String, Json>>,
    pub(super) standard_tools: Option<StandardTools>,
    pub(super) annotations: Option<bool>,
    pub(super) logging_model: Option<String>,
    pub(super) agent: Option<AgentInput>,
    pub(super) refuse_breakdown: Option<bool>,
    pub(super) custom_pricing: Option<bool>,
    pub(super) service_tier: Option<String>,
    pub(super) transcription_seconds: Option<Number>,
    pub(super) ocr: Option<OcrInput>,
    pub(super) events: Option<Vec<IndexMap<String, Json>>>,
}
