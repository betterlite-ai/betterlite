use std::time::SystemTime;

use betterlite_core::cost::batch::batch_cost_per_token;
use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::completion::reading;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_core::cost::realtime::pricing::{RealtimeCostRequest, stream_cost};
use betterlite_core::cost::realtime::transcription::{self, TranscriptionCostRequest};
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::cost_usage::CostUsage;
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
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, resolve};
use rust_decimal::Decimal;

use crate::cost::settings::Settings;
use crate::cost_extra::convert::{
    BatchRequest, ImageRequest, OcrBatchRequest, OcrCostRequest, RealtimeCostWire,
    RealtimeTranscriptionWire, RerankRequest, TranscriptionRequest, VectorStoreRequest,
    VideoRequest,
};
use crate::cost_extra::engine::support::{Capture, numeric, prompt_extras};
use crate::cost_extra::error::{RealtimeFailure, RerankFailure};

pub(crate) mod support;

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
        support::rerank_price(provider, record.as_ref(), request.billed_units.as_ref())
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
