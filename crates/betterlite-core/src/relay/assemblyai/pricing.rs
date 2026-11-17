use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{ASSEMBLY_AI_FALLBACK_MODEL, ASSEMBLY_AI_PROVIDER, AssemblyAiError};

// get_cost_for_assembly_transcript: a missing duration gives no cost, and the rest multiplies out
pub(super) fn transcript_cost(
    transcript: &IndexMap<String, Json>,
    speech_model: &str,
    pricing: &dyn PriceLookup,
) -> Result<Option<Decimal>, AssemblyAiError> {
    let Some(duration) = duration_seconds(transcript)? else {
        return Ok(None);
    };
    let Some(rate) = second_rate(speech_model, pricing)? else {
        return Ok(None);
    };
    duration
        .checked_mul(rate)
        .map(Some)
        .ok_or(AssemblyAiError::CostOverflow)
}

fn duration_seconds(
    transcript: &IndexMap<String, Json>,
) -> Result<Option<Decimal>, AssemblyAiError> {
    match transcript.get("audio_duration") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            parse_decimal_lexeme(&number.to_string())
                .map(Some)
                .map_err(|_source| AssemblyAiError::NumberField {
                    field: "audio_duration",
                })
        }
        Some(_) => Err(AssemblyAiError::NumberField {
            field: "audio_duration",
        }),
    }
}

// get_cost_per_second_for_assembly_model: the speech model's rate, else the assemblyai/nano rate
fn second_rate(
    speech_model: &str,
    pricing: &dyn PriceLookup,
) -> Result<Option<Decimal>, AssemblyAiError> {
    if let Some(rate) = rate_for(speech_model, pricing)? {
        return Ok(Some(rate));
    }
    rate_for(ASSEMBLY_AI_FALLBACK_MODEL, pricing)
}

// A model the price map does not resolve, or an entry without a second rate, falls through
fn rate_for(model: &str, pricing: &dyn PriceLookup) -> Result<Option<Decimal>, AssemblyAiError> {
    let Some(record) = pricing.effective_model_info(model, Some(ASSEMBLY_AI_PROVIDER)) else {
        return Ok(None);
    };
    match record.get("input_cost_per_second") {
        None | Some(Scalar::Null) => Ok(None),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(|_source| AssemblyAiError::NumberField {
                field: "input_cost_per_second",
            }),
        Some(_) => Err(AssemblyAiError::NumberField {
            field: "input_cost_per_second",
        }),
    }
}
