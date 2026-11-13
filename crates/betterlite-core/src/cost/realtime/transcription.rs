use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::Event;
use super::error::RealtimeCostError;

pub mod session;

pub const TRANSCRIPTION_COMPLETED_EVENT: &str =
    "conversation.item.input_audio_transcription.completed";

pub struct TranscriptionCostRequest<'request> {
    pub results: &'request [Event],
    pub custom_llm_provider: &'request str,
    pub litellm_model_name: &'request str,
    pub custom_pricing_model: Option<&'request str>,
}

struct TranscriptionRates<'rates> {
    lookup: &'rates dyn PriceLookup,
    model_info: Option<&'rates PriceRecord>,
    override_info: Option<&'rates PriceRecord>,
}

impl TranscriptionRates<'_> {
    fn for_family(&self, keys: &[&'static str]) -> Result<Decimal, RealtimeCostError> {
        if let Some(found) = declared_rate(self.override_info, keys, self.lookup)? {
            return Ok(found);
        }
        Ok(declared_rate(self.model_info, keys, self.lookup)?.unwrap_or(Decimal::ZERO))
    }
}

// Ports handle_realtime_transcription_cost_calculation: completed events bill independently of response.done.
pub fn cost(
    request: &TranscriptionCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, RealtimeCostError> {
    let completed: Vec<&Event> = request
        .results
        .iter()
        .filter(|event| {
            matches!(
                event.get("type"),
                Some(Json::Text(event_type)) if event_type == TRANSCRIPTION_COMPLETED_EVENT
            )
        })
        .collect();
    if completed.is_empty() {
        return Ok(Decimal::ZERO);
    }
    let model = session::model_name(request.results)?
        .unwrap_or_else(|| request.litellm_model_name.to_owned());
    let model_info = lookup.effective_model_info(&model, Some(request.custom_llm_provider));
    let override_info = request
        .custom_pricing_model
        .and_then(|custom| lookup.effective_model_info(custom, Some(request.custom_llm_provider)));
    let rates = TranscriptionRates {
        lookup,
        model_info: model_info.as_ref(),
        override_info: override_info.as_ref(),
    };
    let mut total = Decimal::ZERO;
    for event in completed {
        let amount = usage_cost(event.get("usage"), &rates)?;
        total = total
            .checked_add(amount)
            .ok_or(RealtimeCostError::Overflow)?;
    }
    Ok(total)
}

fn usage_cost(
    usage: Option<&Json>,
    rates: &TranscriptionRates<'_>,
) -> Result<Decimal, RealtimeCostError> {
    if rates.model_info.is_none() && rates.override_info.is_none() {
        return Ok(Decimal::ZERO);
    }
    let Some(value) = usage else {
        return Ok(Decimal::ZERO);
    };
    match value {
        Json::Null => Ok(Decimal::ZERO),
        Json::Object(fields) => match fields.get("type") {
            Some(Json::Text(usage_type)) if usage_type == "duration" => {
                duration_cost(fields, rates)
            }
            Some(Json::Text(usage_type)) if usage_type == "tokens" => token_cost(fields, rates),
            Some(_) | None => Ok(Decimal::ZERO),
        },
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(RealtimeCostError::Shape {
                field: "usage",
                expected: "an object or null",
            })
        }
    }
}

fn duration_cost(
    fields: &IndexMap<String, Json>,
    rates: &TranscriptionRates<'_>,
) -> Result<Decimal, RealtimeCostError> {
    let seconds = detail_count(Some(fields), "seconds")?;
    let rate = rates.for_family(&["input_cost_per_second"])?;
    seconds.checked_mul(rate).ok_or(RealtimeCostError::Overflow)
}

fn token_cost(
    fields: &IndexMap<String, Json>,
    rates: &TranscriptionRates<'_>,
) -> Result<Decimal, RealtimeCostError> {
    let details = match fields.get("input_token_details") {
        None | Some(Json::Null) => None,
        Some(Json::Object(found)) => Some(found),
        Some(_) => {
            return Err(RealtimeCostError::Shape {
                field: "usage.input_token_details",
                expected: "an object or null",
            });
        }
    };
    let audio = detail_count(details, "audio_tokens")?;
    let text = detail_count(details, "text_tokens")?;
    let output = detail_count(Some(fields), "output_tokens")?;
    let audio_rate = rates.for_family(&["input_cost_per_audio_token", "input_cost_per_token"])?;
    let text_rate = rates.for_family(&["input_cost_per_token"])?;
    let output_rate = rates.for_family(&["output_cost_per_token"])?;
    let audio_cost = audio
        .checked_mul(audio_rate)
        .ok_or(RealtimeCostError::Overflow)?;
    let text_cost = text
        .checked_mul(text_rate)
        .ok_or(RealtimeCostError::Overflow)?;
    let output_cost = output
        .checked_mul(output_rate)
        .ok_or(RealtimeCostError::Overflow)?;
    audio_cost
        .checked_add(text_cost)
        .and_then(|sum| sum.checked_add(output_cost))
        .ok_or(RealtimeCostError::Overflow)
}

fn detail_count(
    fields: Option<&IndexMap<String, Json>>,
    name: &'static str,
) -> Result<Decimal, RealtimeCostError> {
    let Some(found) = fields else {
        return Ok(Decimal::ZERO);
    };
    match found.get(name) {
        None | Some(Json::Null) => Ok(Decimal::ZERO),
        Some(Json::Number(number)) => decimal_number(number, name),
        Some(_) => Err(RealtimeCostError::Shape {
            field: name,
            expected: "a number or null",
        }),
    }
}

// Ports _declared_transcription_rate: only a field the raw catalog entry declares may win, zero included.
fn declared_rate(
    info: Option<&PriceRecord>,
    keys: &[&'static str],
    lookup: &dyn PriceLookup,
) -> Result<Option<Decimal>, RealtimeCostError> {
    let Some(found) = info else {
        return Ok(None);
    };
    let Some(Scalar::Text(key)) = found.get("key") else {
        return Ok(None);
    };
    let Some(raw) = lookup.raw_cost_map_entry(key) else {
        return Ok(None);
    };
    for name in keys.iter().copied() {
        let declared = raw
            .get(name)
            .is_some_and(|value| !matches!(value, Scalar::Null));
        if !declared {
            continue;
        }
        let Some(value) = found.get(name) else {
            continue;
        };
        if matches!(value, Scalar::Null) {
            continue;
        }
        return decimal_scalar(value, name).map(Some);
    }
    Ok(None)
}

fn decimal_scalar(value: &Scalar, field: &'static str) -> Result<Decimal, RealtimeCostError> {
    match value {
        Scalar::Number(number) => decimal_number(number, field),
        Scalar::Null | Scalar::Bool(_) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_) => {
            Err(RealtimeCostError::NotANumber { field })
        }
    }
}

fn decimal_number(number: &Number, field: &'static str) -> Result<Decimal, RealtimeCostError> {
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        Decimal::from_str(&text)
    };
    parsed.map_err(|source| RealtimeCostError::Rate { field, source })
}
