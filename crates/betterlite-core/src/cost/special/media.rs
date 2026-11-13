use std::error::Error;

use betterlite_models::billing_text::count_characters;
use betterlite_models::duration_cost::openai_seconds::video_generation_cost;
use betterlite_models::duration_cost::video::{VideoCostRequest, default_video_cost_calculator};
use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use super::error::SpecialChargeError;
use super::request::{HiddenDuration, SpecialRequest};
use crate::cost::context::pricing::{PricingQuery, deployment_info};
use crate::cost::prepare::{AttemptBilling, RerankUnits};
use crate::routes::CallType;

// The video create, edit and remix calls only; a dictionary response has no usage attribute, so the caller passes none.
pub(super) fn video_charge<ImageError>(
    request: &SpecialRequest<'_>,
    lookup: &dyn PriceLookup,
    model: &str,
    provider: Option<&str>,
) -> Result<Decimal, SpecialChargeError<ImageError>>
where
    ImageError: Error + 'static,
{
    let query = PricingQuery {
        custom_pricing: request.custom_pricing,
        router_model_id: request.router_model_id,
        logging_params: request.logging.params,
    };
    let model_info = deployment_info(&query, lookup)?;
    let usage = request.response.video_usage.as_ref();
    let resolution = usage
        .and_then(|found| found.video_resolution)
        .map(|text| text.trim().to_lowercase());
    let mut cost_request = VideoCostRequest {
        model,
        duration_seconds: Decimal::ZERO,
        custom_llm_provider: provider,
        model_info: model_info.as_ref(),
        video_resolution: resolution.as_deref(),
    };
    if let Some(found) = usage {
        if model_info.is_none()
            && let Some(reported) = found.provider_reported_cost_usd
        {
            return Ok(reported);
        }
        if let Some(duration_seconds) = found.duration_seconds {
            cost_request.duration_seconds = duration_seconds;
            let count = found.video_count.map_or(1, |count| count.max(1));
            return video_generation_cost(&cost_request, lookup)?
                .checked_mul(Decimal::from(count))
                .ok_or(SpecialChargeError::Overflow);
        }
    }
    Ok(default_video_cost_calculator(&cost_request, lookup)?)
}

// Speech, transcription and rerank only record what the ordinary token stage later prices.
pub(super) fn record_media(request: &SpecialRequest<'_>, billing: &mut AttemptBilling) {
    let call_type = request.call_type;
    if matches!(call_type, CallType::Speech | CallType::Aspeech) {
        let characters = count_characters(request.prompt);
        billing.prompt_characters = Some(u64::try_from(characters).unwrap_or(u64::MAX));
    }
    if matches!(
        call_type,
        CallType::Transcription | CallType::Atranscription
    ) {
        let transcription = &request.response.transcription;
        billing.transcription_duration = match transcription.hidden {
            HiddenDuration::Seconds(seconds) => seconds,
            HiddenDuration::Null => Decimal::ZERO,
            HiddenDuration::Missing => transcription.response_duration.unwrap_or_default(),
        };
    }
    if matches!(call_type, CallType::Rerank | CallType::Arerank)
        && let Some(units) = &request.response.rerank
    {
        billing.rerank_units = Some(RerankUnits {
            search_units: units.search_units,
            total_tokens: units.total_tokens,
        });
        // Cohere charges per request by default, and a zero count falls back like a missing one.
        billing.completion_tokens = units.search_units.filter(|found| *found != 0).unwrap_or(1);
    }
}
