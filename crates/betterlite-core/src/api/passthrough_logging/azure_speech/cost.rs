use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use super::duration::billed_audio_seconds;
use super::route::SpeechRoute;
use super::{AzureSpeechLoggingError, PRICING_MODEL, PRICING_PROVIDER};

// The price comes from the fixed Azure STT entry, never from the route-derived name, and a missing one bills zero.
pub(super) fn response_cost(
    route: SpeechRoute,
    request_body: Option<&[u8]>,
    response_body: &[u8],
    lookup: &dyn PriceLookup,
) -> Result<Decimal, AzureSpeechLoggingError> {
    let seconds = billed_audio_seconds(route, request_body, response_body)?;
    if seconds <= Decimal::ZERO {
        return Ok(Decimal::ZERO);
    }
    match cost_per_second(PRICING_MODEL, Some(PRICING_PROVIDER), seconds, lookup) {
        Ok((prompt, completion)) => prompt
            .checked_add(completion)
            .ok_or(AzureSpeechLoggingError::CostRange),
        Err(error) => {
            tracing::warn!(
                model = PRICING_MODEL,
                %error,
                "no price for the Azure Speech pricing model, logging the call at zero cost"
            );
            Ok(Decimal::ZERO)
        }
    }
}
