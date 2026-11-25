use std::str::FromStr as _;
use std::time::SystemTime;

use betterlite_core::cost::batch::{BatchCostRequest, batch_cost_per_token, error::BatchCostError};
use betterlite_core::cost::completion::reading;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_core::cost::realtime::pricing::{RealtimeCostRequest, stream_cost};
use betterlite_core::cost::realtime::transcription;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::duration_cost::video::default_video_cost_calculator;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::image_cost::default_calculator::default_image_cost_calculator;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::ocr_pricing::{OcrCostError, ocr_batch_cost, ocr_cost};
use betterlite_models::price_lookup::PriceLookup as _;
use betterlite_models::resolution::{ResolutionRequest, resolve};
use betterlite_models::usage_sum::Usage;
use rust_decimal::Decimal;

use crate::cost::convert::Amount;
use crate::cost::settings::Settings;
use crate::cost_extra::convert::{
    BatchRequest, ImageRequest, OcrRequest, Pair, RealtimeCostWire, RealtimeTranscriptionWire,
    RerankRequest, TranscriptionRequest, VectorStoreRequest, VideoRequest,
};
use crate::cost_extra::engine::support::{Capture, numeric, prompt_extras};
use crate::cost_extra::error::{RealtimeFailure, RerankFailure};

pub(crate) mod support;

mod tests;

const DEFAULT_VECTOR_PROVIDER: &str = "openai";
const VERTEX_SEARCH_API: &str = "search_api";
const VERTEX_SEARCH_MODEL: &str = "vertex_ai/search_api";
const QUERY_RATE: &str = "input_cost_per_query";

// providers.json registers these slugs, but litellm.types.utils.LlmProviders does not, and LlmProviders() rejects them.
#[rustfmt::skip]
const REGISTRY_ONLY_PROVIDERS: [ProviderId; 8] = [
    ProviderId::Abliteration, ProviderId::Aihubmix, ProviderId::Crusoe, ProviderId::Empiriolabs,
    ProviderId::Gmi, ProviderId::Llamagate, ProviderId::Sarvam, ProviderId::Veniceai,
];

// LlmProviders(custom_llm_provider): an unknown or registry-only name is the ValueError Python's enum raises.
fn litellm_provider(provider: &str) -> Result<ProviderId, RerankFailure> {
    let unknown = || RerankFailure::UnknownProvider {
        provider: provider.to_owned(),
    };
    let known = ProviderId::from_str(provider).map_err(|_unparsed| unknown())?;
    if REGISTRY_ONLY_PROVIDERS.contains(&known) {
        return Err(unknown());
    }
    Ok(known)
}

// One extra-cost resource: the price map and the zone database every specialised calculator reads.
pub(crate) struct Engine {
    pub(crate) settings: Settings,
}

impl Engine {
    pub(crate) fn ocr_cost(&self, request: &OcrRequest) -> Result<(Amount, Amount), OcrCostError> {
        let billing = request.billing();
        let info = request.model_info.as_ref();
        let lookup = &self.settings.lookup;
        let provider = request.custom_llm_provider.as_deref();
        ocr_cost(&request.model, provider, &billing, info, lookup)
            .map(|cost| (Amount(cost), Amount(Decimal::ZERO)))
    }

    pub(crate) fn ocr_batch_cost(&self, request: &OcrRequest) -> Result<Pair, OcrCostError> {
        ocr_batch_cost(
            &request.model,
            request.custom_llm_provider.as_deref(),
            &request.batch_usage(),
            request.model_info.as_ref(),
            &self.settings.lookup,
        )
        .map(|cost| (Amount(cost), Amount(Decimal::ZERO)))
    }

    // vector_store_search_cost: only vertex search_api declares a price, and it is the catalog's per-query rate.
    pub(crate) fn vector_store_cost(
        &self,
        request: &VectorStoreRequest,
    ) -> Result<Pair, RerankFailure> {
        let lookup = &self.settings.lookup;
        let resolution = lookup.resolution();
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
        let fallback = request
            .custom_llm_provider
            .as_deref()
            .unwrap_or(DEFAULT_VECTOR_PROVIDER);
        let provider = resolved
            .as_ref()
            .map_or_else(|| fallback, |found| found.provider.slug());
        let api_type = resolved.as_ref().map(|found| found.model.as_str());
        let known = litellm_provider(provider)?;
        if known != ProviderId::VertexAi || api_type != Some(VERTEX_SEARCH_API) {
            return Ok((Amount(Decimal::ZERO), Amount(Decimal::ZERO)));
        }
        let record = lookup.effective_model_info(VERTEX_SEARCH_MODEL, None);
        let price = numeric(record.as_ref(), QUERY_RATE)?.unwrap_or_default();
        Ok((Amount(price), Amount(Decimal::ZERO)))
    }

    pub(crate) fn rerank_cost(&self, request: &RerankRequest) -> Result<Pair, RerankFailure> {
        let lookup = &self.settings.lookup;
        let resolution = lookup.resolution();
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
        let record = lookup.effective_model_info(&request.model, Some(provider));
        let units = request.billed_units.as_ref();
        support::rerank_price(provider, record.as_ref(), units)
            .map(|(prompt, completion)| (Amount(prompt), Amount(completion)))
    }

    pub(crate) fn transcription_cost(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<(Amount, Amount), DurationCostError> {
        let provider = request.custom_llm_provider.as_deref();
        let duration = request.duration.0;
        let lookup = &self.settings.lookup;
        cost_per_second(&request.model, provider, duration, lookup)
            .map(|(prompt, completion)| (Amount(prompt), Amount(completion)))
    }

    pub(crate) fn image_cost(&self, request: &ImageRequest) -> Result<Amount, ImageCostError> {
        default_image_cost_calculator(&ImageRequest::calculator(request), &self.settings.lookup)
            .map(Amount)
    }

    pub(crate) fn video_cost(&self, request: &VideoRequest) -> Result<Amount, DurationCostError> {
        default_video_cost_calculator(&VideoRequest::calculator(request), &self.settings.lookup)
            .map(Amount)
    }

    pub(crate) fn batch_cost(&self, request: &BatchRequest) -> Result<Pair, BatchCostError> {
        let usage = Usage::from(&request.usage);
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
        Ok((Amount(priced.prompt_cost), Amount(priced.completion_cost)))
    }

    pub(crate) fn realtime_cost<Clock>(
        &self,
        request: &RealtimeCostWire,
        clock: &mut Clock,
    ) -> Result<(Amount, Option<CostBreakdown>), RealtimeFailure>
    where
        Clock: FnMut() -> SystemTime,
    {
        let usage = Usage::from(&request.combined_usage);
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
                moment: reading(clock)?,
            },
            &self.settings.lookup,
            &self.settings.database,
            Some(&mut capture),
        )?;
        Ok((Amount(cost), capture.breakdown))
    }

    pub(crate) fn realtime_transcription_cost(
        &self,
        request: &RealtimeTranscriptionWire,
    ) -> Result<Amount, RealtimeCostError> {
        let events = RealtimeTranscriptionWire::calculator(request);
        let cost = transcription::cost(&events, &self.settings.lookup)?;
        Ok(Amount(cost))
    }
}
