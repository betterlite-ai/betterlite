use betterlite_core::cost::realtime::{Event, transcription::TranscriptionCostRequest};
use betterlite_models::decisions::Json;
use betterlite_models::duration_cost::video::VideoCostRequest;
use betterlite_models::image_cost::default_calculator::DefaultImageCostRequest;
use betterlite_models::ocr_pricing::{OcrBatchUsage, OcrBilling, OcrResponseKind, OcrUsage};
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use serde::Deserialize;

use crate::cost::convert::{Amount, optional};

pub(crate) mod usage;

mod tests;

use usage::UsageWire;

// The calculators read borrowed request structs; one field list per wire type builds each of them.
macro_rules! calculator_request {
    ($wire:ident => $target:ident, $request:ident { $($field:ident: $value:expr),+ $(,)? }) => {
        impl $wire {
            pub(crate) fn calculator($request: &Self) -> $target<'_> { $target { $($field: $value),+ } }
        }
    };
}

// Both OCR entry points take a model, its provider, LiteLLM's OCRUsageInfo and a deployment's own prices.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrRequest {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) usage: Option<OcrUsageWire>,
    pub(crate) model_info: Option<PriceRecord>,
}

// Every counter of LiteLLM's OCRUsageInfo is optional and absent means zero; other counters go unread.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct OcrUsageWire {
    pub(crate) credits: Option<Amount>,
    pub(crate) pages_processed: Option<i64>,
    pub(crate) pages_processed_annotation: Option<i64>,
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

impl OcrRequest {
    pub(crate) fn billing(&self) -> OcrBilling {
        OcrBilling {
            kind: OcrResponseKind::Ocr,
            usage: self.usage.as_ref().map(OcrUsage::from),
        }
    }

    pub(crate) fn batch_usage(&self) -> OcrBatchUsage {
        let usage = self.usage.as_ref();
        OcrBatchUsage {
            pages_processed: usage.and_then(|fields| fields.pages_processed).unwrap_or(0),
            pages_processed_annotation: usage
                .and_then(|fields| fields.pages_processed_annotation)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorStoreRequest {
    pub(crate) model: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
    // The search response carries no price of its own; LiteLLM reads only the model and provider.
    #[serde(rename = "response")]
    _response: Json,
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
    #[serde(rename = "n")]
    pub(crate) n_images: Option<i64>,
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

calculator_request!(ImageRequest => DefaultImageCostRequest, wire {
    model: &wire.model, custom_llm_provider: wire.custom_llm_provider.as_deref(),
    quality: wire.quality.as_deref(), n_images: wire.n_images, size: wire.size.as_deref(),
    model_info: wire.model_info.as_ref(),
});

calculator_request!(VideoRequest => VideoCostRequest, wire {
    model: &wire.model, duration_seconds: wire.duration_seconds.0,
    custom_llm_provider: wire.custom_llm_provider.as_deref(),
    model_info: wire.model_info.as_ref(), video_resolution: wire.video_resolution.as_deref(),
});

calculator_request!(RealtimeTranscriptionWire => TranscriptionCostRequest, wire {
    results: &wire.results, custom_llm_provider: &wire.custom_llm_provider,
    litellm_model_name: &wire.litellm_model_name,
    custom_pricing_model: wire.custom_pricing_model.as_deref(),
});

pub(crate) type Pair = (Amount, Amount);
