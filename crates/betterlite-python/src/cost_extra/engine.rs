use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::SystemTime;

use betterlite_core::cost::batch::batch_cost_per_token;
use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::completion::reading;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_core::cost::realtime::pricing::{RealtimeCostRequest, stream_cost};
use betterlite_core::cost::realtime::transcription::{self, TranscriptionCostRequest};
use betterlite_core::cost::realtime::{Event, realtime_stream, responses_ws};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::modality_projection::CachedTokenCounters;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::duration_cost::video::{VideoCostRequest, default_video_cost_calculator};
use betterlite_models::image_cost::default_calculator::{
    DefaultImageCostRequest, default_image_cost_calculator,
};
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::ocr_pricing::{
    OcrBatchUsage, OcrBilling, OcrCostError, OcrResponseKind, OcrUsage, ocr_batch_cost, ocr_cost,
};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, resolve};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageSumError, usage_sum};
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use rust_decimal::Decimal;

use crate::cost_extra::convert::{
    BatchRequest, CollectUsageRequest, CombineUsageRequest, ImageRequest, OcrBatchRequest,
    OcrCostRequest, PartitionUsageRequest, RealtimeCostWire, RealtimeTranscriptionWire,
    RerankRequest, TranscriptionRequest, UsageSource, UsageWire, VectorStoreRequest, VideoRequest,
};
use crate::cost_extra::error::{RealtimeFailure, RerankFailure};
use crate::cost_extra::settings::Settings;

mod tests;

const DEFAULT_VECTOR_PROVIDER: &str = "openai";
const VERTEX_PROVIDER: &str = "vertex_ai";
const VERTEX_SEARCH_API: &str = "search_api";
const VERTEX_SEARCH_MODEL: &str = "vertex_ai/search_api";
const QUERY_RATE: &str = "input_cost_per_query";
const TOKEN_RATE: &str = "input_cost_per_token";
const JINA_PROVIDER: &str = "jina_ai";
const VOYAGE_PROVIDER: &str = "voyage";

// One extra-cost resource: the price map and the zone database every specialised calculator reads.
pub(crate) struct Engine {
    pub(crate) settings: Settings,
}

// The breakdown a realtime session leaves for Python; LiteLLM's own logging object keeps it.
#[derive(Debug, Default)]
pub(crate) struct Capture {
    pub(crate) breakdown: Option<CostBreakdown>,
}

impl CostBreakdownTarget for Capture {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.breakdown = Some(CostBreakdown {
            service_tier: input.service_tier.clone(),
            data_residency: input.data_residency.clone(),
            vertex_location: input.vertex_location.clone(),
            input_cost: Some(input.input_cost),
            cache_read_cost: input.cache_read_cost,
            cache_creation_cost: input.cache_creation_cost,
            output_cost: Some(input.output_cost),
            reasoning_cost: input.reasoning_cost,
            total_cost: Some(input.total_cost),
            tool_usage_cost: Some(input.tool_usage_cost),
            guardrail_cost: None,
            additional_costs: input
                .additional_costs
                .clone()
                .filter(|costs| !costs.is_empty()),
            original_cost: input.original_cost,
            discount_percent: input.discount_percent,
            discount_amount: input.discount_amount,
            margin_percent: input.margin_percent,
            margin_fixed_amount: input.margin_fixed_amount,
            margin_total_amount: input.margin_total_amount,
            extra: BTreeMap::new(),
        });
        Ok(())
    }
}

impl Engine {
    // ocr_cost: credits first, then the page and annotation families, all read off the layered prices.
    pub(crate) fn ocr_cost(&self, request: &OcrCostRequest) -> Result<Decimal, OcrCostError> {
        let billing = OcrBilling {
            kind: OcrResponseKind::Ocr,
            usage: request.usage.as_ref().map(OcrUsage::from),
        };
        ocr_cost(
            &request.model,
            request.custom_llm_provider.as_deref(),
            &billing,
            request.model_info.as_ref(),
            &self.settings.lookup,
        )
    }

    pub(crate) fn ocr_batch_cost(
        &self,
        request: &OcrBatchRequest,
    ) -> Result<Decimal, OcrCostError> {
        let usage = OcrBatchUsage {
            pages_processed: request.usage.pages_processed.unwrap_or(0),
            pages_processed_annotation: request.usage.pages_processed_annotation.unwrap_or(0),
        };
        ocr_batch_cost(
            &request.model,
            request.custom_llm_provider.as_deref(),
            &usage,
            request.model_info.as_ref(),
            &self.settings.lookup,
        )
    }

    // vector_store_search_cost: only vertex search_api declares a price, and it is the catalog's own per-query rate.
    pub(crate) fn vector_store_cost(
        &self,
        request: &VectorStoreRequest,
    ) -> Result<(Decimal, Decimal), ResolutionError> {
        let resolution = self.settings.lookup.resolution();
        let resolved = request
            .model
            .as_ref()
            .filter(|name| name.contains('/'))
            .map(|slashed| {
                resolve(
                    &ResolutionRequest {
                        model: slashed,
                        custom_llm_provider: None,
                        api_base: None,
                        api_key: None,
                    },
                    &resolution,
                )
            })
            .transpose()?;
        let provider = resolved.as_ref().map_or_else(
            || {
                request
                    .custom_llm_provider
                    .as_deref()
                    .unwrap_or(DEFAULT_VECTOR_PROVIDER)
            },
            |found| found.provider.slug(),
        );
        let api_type = resolved.as_ref().map(|found| found.model.as_str());
        if provider != VERTEX_PROVIDER || api_type != Some(VERTEX_SEARCH_API) {
            return Ok((Decimal::ZERO, Decimal::ZERO));
        }
        let record = self
            .settings
            .lookup
            .effective_model_info(VERTEX_SEARCH_MODEL, None);
        let price = match numeric(record.as_ref(), QUERY_RATE)? {
            Some(found) => found,
            None => Decimal::ZERO,
        };
        Ok((price, Decimal::ZERO))
    }

    // rerank_cost: the provider is resolved again and its own formula reads the billed units.
    pub(crate) fn rerank_cost(
        &self,
        request: &RerankRequest,
    ) -> Result<(Decimal, Decimal), RerankFailure> {
        let resolution = self.settings.lookup.resolution();
        let resolved = resolve(
            &ResolutionRequest {
                model: &request.model,
                custom_llm_provider: request.custom_llm_provider.as_deref(),
                api_base: None,
                api_key: None,
            },
            &resolution,
        )?;
        let provider = resolved.provider.slug();
        let record = self
            .settings
            .lookup
            .effective_model_info(&request.model, Some(provider));
        rerank_price(provider, record.as_ref(), request.billed_units.as_ref())
    }

    pub(crate) fn transcription_cost(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<(Decimal, Decimal), DurationCostError> {
        cost_per_second(
            &request.model,
            request.custom_llm_provider.as_deref(),
            request.duration.0,
            &self.settings.lookup,
        )
    }

    pub(crate) fn image_cost(&self, request: &ImageRequest) -> Result<Decimal, ImageCostError> {
        default_image_cost_calculator(
            &DefaultImageCostRequest {
                model: &request.model,
                custom_llm_provider: request.custom_llm_provider.as_deref(),
                quality: request.quality.as_deref(),
                n_images: request.n,
                size: request.size.as_deref(),
                model_info: request.model_info.as_ref(),
            },
            &self.settings.lookup,
        )
    }

    pub(crate) fn video_cost(&self, request: &VideoRequest) -> Result<Decimal, DurationCostError> {
        default_video_cost_calculator(
            &VideoCostRequest {
                model: &request.model,
                duration_seconds: request.duration_seconds.0,
                custom_llm_provider: request.custom_llm_provider.as_deref(),
                model_info: request.model_info.as_ref(),
                video_resolution: request.video_resolution.as_deref(),
            },
            &self.settings.lookup,
        )
    }

    // batch_cost_calculator: the batch rates, the half-rate fallback and the regional uplift, in one call.
    pub(crate) fn batch_cost(
        &self,
        request: &BatchRequest,
    ) -> Result<(Decimal, Decimal), BatchCostError> {
        let usage = request.usage.usage();
        let cost_usage = CostUsage::from(&usage);
        let extras = prompt_extras(usage.prompt_tokens_details.as_ref());
        let resolution = self.settings.lookup.resolution();
        let priced = batch_cost_per_token(
            &BatchCostRequest {
                model: &request.model,
                custom_llm_provider: request.custom_llm_provider.as_deref(),
                usage: &cost_usage,
                prompt_extras: &extras,
                supplied_model_info: request.model_info.as_ref(),
                data_residency: request.data_residency.as_deref(),
            },
            &self.settings.lookup,
            &resolution,
        )?;
        Ok((priced.prompt_cost, priced.completion_cost))
    }

    // handle_realtime_stream_cost_calculation: the session's token costs, its transcription charge and the breakdown.
    pub(crate) fn realtime_cost(
        &self,
        request: &RealtimeCostWire,
    ) -> Result<(Decimal, Option<CostBreakdown>), RealtimeFailure> {
        let usage = request.combined_usage.usage();
        let mut capture = Capture::default();
        let cost = stream_cost(
            &RealtimeCostRequest {
                results: &request.results,
                combined_usage: &usage,
                custom_llm_provider: &request.custom_llm_provider,
                litellm_model_name: &request.litellm_model_name,
                data_residency: request.data_residency.as_deref(),
                custom_pricing_model: request.custom_pricing_model.as_deref(),
                base_pricing_model: request.base_pricing_model.as_deref(),
                moment: reading(&mut SystemTime::now)?,
            },
            &self.settings.lookup,
            &self.settings.database,
            Some(&mut capture),
        )?;
        Ok((cost, capture.breakdown))
    }

    // handle_realtime_transcription_cost_calculation: each completed transcription event bills on its own.
    pub(crate) fn realtime_transcription_cost(
        &self,
        request: &RealtimeTranscriptionWire,
    ) -> Result<Decimal, RealtimeCostError> {
        transcription::cost(
            &TranscriptionCostRequest {
                results: &request.results,
                custom_llm_provider: &request.custom_llm_provider,
                litellm_model_name: &request.litellm_model_name,
                custom_pricing_model: request.custom_pricing_model.as_deref(),
            },
            &self.settings.lookup,
        )
    }
}

// combine_usage_objects: every counter and detail summed, an empty list giving LiteLLM's default usage.
pub(crate) fn combine_usage(request: &CombineUsageRequest) -> Result<Usage, UsageSumError> {
    let usages = request
        .usages
        .iter()
        .map(UsageWire::usage)
        .collect::<Vec<Usage>>();
    usage_sum(&usages)
}

// The two collect_usage methods differ only in the event types they bill.
pub(crate) fn collect_usage(
    request: &CollectUsageRequest,
) -> Result<Vec<Usage>, RealtimeCostError> {
    match request.source {
        UsageSource::Realtime => realtime_stream::collect_usage(&request.results),
        UsageSource::ResponsesWs => responses_ws::collect_usage(&request.results),
    }
}

pub(crate) fn partition_usage(
    request: &PartitionUsageRequest,
) -> Result<Vec<(Option<String>, Vec<Event>)>, RealtimeCostError> {
    Ok(responses_ws::partition_by_service_tier(&request.results)?
        .into_iter()
        .collect())
}

// Ports jina's and voyage's rerank override; every other provider bills the query units.
fn rerank_price(
    provider: &str,
    record: Option<&PriceRecord>,
    billed_units: Option<&RerankBilledUnits>,
) -> Result<(Decimal, Decimal), RerankFailure> {
    let Some(units) = billed_units else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let (key, units) = match provider {
        JINA_PROVIDER | VOYAGE_PROVIDER => (TOKEN_RATE, units.total_tokens.flatten()),
        _ => (QUERY_RATE, units.search_units.flatten()),
    };
    let (Some(rate), Some(count)) = (numeric(record, key)?, units) else {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    };
    let cost = rate
        .checked_mul(Decimal::from(count))
        .ok_or(RerankFailure::Overflow)?;
    Ok((cost, Decimal::ZERO))
}

fn numeric(record: Option<&PriceRecord>, key: &str) -> Result<Option<Decimal>, RerankFailure> {
    match record.and_then(|found| found.get(key)) {
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(RerankFailure::Rate),
        Some(Scalar::Bool(flag)) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_)) | None => {
            Ok(None)
        }
    }
}

// The prompt details batch pricing reads besides the counters, spelled as batch_cost_calculator passes them.
fn prompt_extras(details: Option<&PromptTokensDetails>) -> PromptExtras {
    let Some(details) = details else {
        return PromptExtras::default();
    };
    PromptExtras {
        cache_write_tokens: details.cache_write_tokens,
        cache_creation_tokens: details.cache_creation_tokens,
        cache_creation_token_details: None,
        cached_tokens_details: details.cached_tokens_details.as_ref().map(|cached| {
            CachedTokenCounters {
                text_tokens: cached.text_tokens,
                audio_tokens: cached.audio_tokens,
                image_tokens: cached.image_tokens,
            }
        }),
        character_count: details.character_count,
        image_count: details.image_count,
        video_length_seconds: details.video_length_seconds,
        audio_length_seconds: details.audio_length_seconds,
        query_count: details.query_count,
    }
}
