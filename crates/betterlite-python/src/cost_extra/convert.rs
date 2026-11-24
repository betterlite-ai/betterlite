use betterlite_core::cost::realtime::Event;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::OcrUsage;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use serde::{Deserialize, Serialize};

use crate::cost::convert::{Amount, optional};

pub(crate) mod usage;

mod tests;

use usage::UsageWire;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrCostRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) usage: Option<OcrUsageWire>,
    pub(crate) model_info: Option<PriceRecord>,
}

// Both OCR entry points take LiteLLM's OCRUsageInfo: every counter is optional and absent means zero.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrUsageWire {
    #[serde(default)]
    pub(crate) credits: Option<Amount>,
    #[serde(default)]
    pub(crate) pages_processed: Option<i64>,
    #[serde(default)]
    pub(crate) pages_processed_annotation: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrBatchRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) usage: OcrUsageWire,
    pub(crate) model_info: Option<PriceRecord>,
}

impl From<&OcrUsageWire> for OcrUsage {
    fn from(wire: &OcrUsageWire) -> Self {
        Self {
            credits: optional(wire.credits),
            pages_processed: wire.pages_processed,
            pages_processed_annotation: wire.pages_processed_annotation.unwrap_or(0),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorStoreRequest {
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    // The search response carries no price of its own; LiteLLM reads only the model and provider.
    pub(crate) response: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RerankRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) billed_units: Option<RerankBilledUnits>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranscriptionRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) duration: Amount,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) quality: Option<String>,
    pub(crate) n: Option<i64>,
    pub(crate) size: Option<String>,
    pub(crate) model_info: Option<PriceRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VideoRequest {
    pub(crate) model: String,
    pub(crate) duration_seconds: Amount,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) model_info: Option<PriceRecord>,
    pub(crate) video_resolution: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchRequest {
    pub(crate) usage: UsageWire,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) model_info: Option<PriceRecord>,
    pub(crate) data_residency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CombineUsageRequest {
    pub(crate) usages: Vec<UsageWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CollectUsageRequest {
    pub(crate) source: UsageSource,
    pub(crate) results: Vec<Event>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UsageSource {
    Realtime,
    ResponsesWs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartitionUsageRequest {
    pub(crate) results: Vec<Event>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RealtimeCostWire {
    pub(crate) results: Vec<Event>,
    pub(crate) combined_usage: UsageWire,
    pub(crate) custom_llm_provider: String,
    pub(crate) litellm_model_name: String,
    pub(crate) data_residency: Option<String>,
    pub(crate) custom_pricing_model: Option<String>,
    pub(crate) base_pricing_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RealtimeTranscriptionWire {
    pub(crate) results: Vec<Event>,
    pub(crate) custom_llm_provider: String,
    pub(crate) litellm_model_name: String,
    pub(crate) custom_pricing_model: Option<String>,
}

// What a realtime handler answers: the session cost and the breakdown LiteLLM stores on its logging object.
#[derive(Debug, Serialize)]
pub(crate) struct RealtimeCostAnswer<'answer> {
    pub(crate) cost: Amount,
    pub(crate) breakdown: Option<&'answer CostBreakdown>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CostAnswer {
    pub(crate) cost: Amount,
}
