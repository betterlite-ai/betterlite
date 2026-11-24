use std::collections::BTreeMap;

use betterlite_core::cost::realtime::Event;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::OcrUsage;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::usage_sum::{
    CachedTokensDetails, CompletionTokensDetails, PromptTokensDetails, Usage, UsageNumber,
};
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod tests;

// One exact money amount as a JSON number: the lexeme keeps every digit LiteLLM's float would round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Amount(
    #[serde(with = "rust_decimal::serde::arbitrary_precision")] pub(crate) Decimal,
);

pub(crate) const fn money(value: Option<Amount>) -> Option<Decimal> {
    match value {
        Some(inner) => Some(inner.0),
        None => None,
    }
}

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
            credits: money(wire.credits),
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

// The usage a Python Usage object carries: its three counters, the two detail objects and any numeric extra.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UsageWire {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<PromptDetailsWire>,
    pub(crate) completion_tokens_details: Option<CompletionDetailsWire>,
    pub(crate) numeric_extras: BTreeMap<String, UsageNumberView>,
}

impl UsageWire {
    pub(crate) fn usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            prompt_tokens_details: self
                .prompt_tokens_details
                .as_ref()
                .map(PromptTokensDetails::from),
            completion_tokens_details: self
                .completion_tokens_details
                .as_ref()
                .map(CompletionTokensDetails::from),
            numeric_extras: self
                .numeric_extras
                .iter()
                .map(|(name, number)| (name.clone(), number.number()))
                .collect(),
        }
    }
}

// Every counter LiteLLM's prompt wrapper declares; the wrapper allows extras, so unknown keys are ignored.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PromptDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
    pub(crate) video_tokens: Option<i64>,
    pub(crate) web_search_requests: Option<i64>,
    pub(crate) google_maps_grounding_requests: Option<i64>,
    pub(crate) tool_use_tokens: Option<i64>,
    pub(crate) character_count: Option<i64>,
    pub(crate) image_count: Option<i64>,
    pub(crate) video_length_seconds: Option<Amount>,
    pub(crate) audio_length_seconds: Option<Amount>,
    pub(crate) query_count: Option<i64>,
    pub(crate) cache_write_tokens: Option<i64>,
    pub(crate) cache_creation_tokens: Option<i64>,
    pub(crate) cached_tokens: Option<i64>,
    pub(crate) cached_tokens_details: Option<CachedDetailsWire>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CachedDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CompletionDetailsWire {
    pub(crate) text_tokens: Option<i64>,
    pub(crate) audio_tokens: Option<i64>,
    pub(crate) image_tokens: Option<i64>,
    pub(crate) video_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) accepted_prediction_tokens: Option<i64>,
    pub(crate) rejected_prediction_tokens: Option<i64>,
}

impl From<&PromptDetailsWire> for PromptTokensDetails {
    fn from(wire: &PromptDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
            video_tokens: wire.video_tokens,
            web_search_requests: wire.web_search_requests,
            google_maps_grounding_requests: wire.google_maps_grounding_requests,
            tool_use_tokens: wire.tool_use_tokens,
            character_count: wire.character_count,
            image_count: wire.image_count,
            video_length_seconds: money(wire.video_length_seconds),
            audio_length_seconds: money(wire.audio_length_seconds),
            query_count: wire.query_count,
            cache_write_tokens: wire.cache_write_tokens,
            cache_creation_tokens: wire.cache_creation_tokens,
            cached_tokens: wire.cached_tokens,
            cached_tokens_details: wire
                .cached_tokens_details
                .as_ref()
                .map(CachedTokensDetails::from),
        }
    }
}

impl From<&PromptTokensDetails> for PromptDetailsWire {
    fn from(details: &PromptTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            web_search_requests: details.web_search_requests,
            google_maps_grounding_requests: details.google_maps_grounding_requests,
            tool_use_tokens: details.tool_use_tokens,
            character_count: details.character_count,
            image_count: details.image_count,
            video_length_seconds: details.video_length_seconds.map(Amount),
            audio_length_seconds: details.audio_length_seconds.map(Amount),
            query_count: details.query_count,
            cache_write_tokens: details.cache_write_tokens,
            cache_creation_tokens: details.cache_creation_tokens,
            cached_tokens: details.cached_tokens,
            cached_tokens_details: details
                .cached_tokens_details
                .as_ref()
                .map(CachedDetailsWire::from),
        }
    }
}

impl From<&CachedDetailsWire> for CachedTokensDetails {
    fn from(wire: &CachedDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
        }
    }
}

impl From<&CachedTokensDetails> for CachedDetailsWire {
    fn from(details: &CachedTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
        }
    }
}

impl From<&CompletionDetailsWire> for CompletionTokensDetails {
    fn from(wire: &CompletionDetailsWire) -> Self {
        Self {
            text_tokens: wire.text_tokens,
            audio_tokens: wire.audio_tokens,
            image_tokens: wire.image_tokens,
            video_tokens: wire.video_tokens,
            reasoning_tokens: wire.reasoning_tokens,
            accepted_prediction_tokens: wire.accepted_prediction_tokens,
            rejected_prediction_tokens: wire.rejected_prediction_tokens,
        }
    }
}

impl From<&CompletionTokensDetails> for CompletionDetailsWire {
    fn from(details: &CompletionTokensDetails) -> Self {
        Self {
            text_tokens: details.text_tokens,
            audio_tokens: details.audio_tokens,
            image_tokens: details.image_tokens,
            video_tokens: details.video_tokens,
            reasoning_tokens: details.reasoning_tokens,
            accepted_prediction_tokens: details.accepted_prediction_tokens,
            rejected_prediction_tokens: details.rejected_prediction_tokens,
        }
    }
}

// The combined usage as LiteLLM's Usage model reads it back, aliases and numeric extras included.
#[derive(Debug, Serialize)]
pub(crate) struct UsageView {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<PromptDetailsWire>,
    pub(crate) completion_tokens_details: Option<CompletionDetailsWire>,
    // LiteLLM's combined Usage carries an extra as a plain attribute, so the answer spells them flat.
    #[serde(flatten)]
    pub(crate) numeric_extras: BTreeMap<String, UsageNumberView>,
}

// One usage extra both ways: an int, an exact decimal or a flag, as LiteLLM's numeric attributes allow.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum UsageNumberView {
    Integer(i64),
    Amount(Amount),
    Boolean(bool),
}

impl UsageNumberView {
    fn number(&self) -> UsageNumber {
        match self {
            Self::Integer(value) => UsageNumber::Integer(*value),
            Self::Amount(value) => UsageNumber::Decimal(value.0),
            Self::Boolean(flag) => UsageNumber::Boolean(*flag),
        }
    }
}

impl From<&Usage> for UsageView {
    fn from(usage: &Usage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: usage
                .prompt_tokens_details
                .as_ref()
                .map(PromptDetailsWire::from),
            completion_tokens_details: usage
                .completion_tokens_details
                .as_ref()
                .map(CompletionDetailsWire::from),
            numeric_extras: usage
                .numeric_extras
                .iter()
                .map(|(name, number)| (name.clone(), UsageNumberView::from(number)))
                .collect(),
        }
    }
}

impl From<&UsageNumber> for UsageNumberView {
    fn from(number: &UsageNumber) -> Self {
        match number {
            UsageNumber::Integer(value) => Self::Integer(*value),
            UsageNumber::Decimal(value) => Self::Amount(Amount(*value)),
            UsageNumber::Boolean(flag) => Self::Boolean(*flag),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct UsageAnswer {
    pub(crate) usage: UsageView,
}

#[derive(Debug, Serialize)]
pub(crate) struct UsagesAnswer {
    pub(crate) usages: Vec<UsageView>,
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

#[derive(Debug, Serialize)]
pub(crate) struct PartitionAnswer {
    pub(crate) tiers: Vec<ServiceTierGroup>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ServiceTierGroup {
    pub(crate) service_tier: Option<String>,
    pub(crate) events: Vec<Event>,
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

// The one boundary every native method crosses: the text into the shape that method takes.
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn encode<T>(value: &T) -> Result<String, ConvertError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_string(value).map_err(|source| ConvertError::Encode { source })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
