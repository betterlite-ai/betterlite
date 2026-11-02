use rust_decimal::Decimal;
use unicode_general_category::{GeneralCategory, get_general_category};

use super::error::DurationCostError;
use super::numeric::numeric_price;
use crate::price_lookup::{PriceLookup, PriceRecord};

// Mirrors Python's str.isalnum(): the Lu/Ll/Lt/Lm/Lo letter categories or the Nd/Nl/No number categories, not Unicode's wider Alphabetic/Numeric properties (which also keep combining marks and circled letters).
fn is_alnum(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::DecimalNumber
            | GeneralCategory::LetterNumber
            | GeneralCategory::OtherNumber
    )
}

// Trims, lowercases and keeps only Unicode alphanumerics and underscores, so "1,080p" selects "1080p"; an empty or over-long result means no resolution-specific field applies.
#[must_use]
pub fn sanitize_video_resolution(resolution: &str) -> Option<String> {
    let trimmed = resolution.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    let safe: String = trimmed
        .chars()
        .filter(|character| is_alnum(*character) || *character == '_')
        .collect();
    if safe.is_empty() || safe.chars().count() > 24 {
        return None;
    }
    Some(safe)
}

// A nonnull resolution-specific rate overrides the base rate, including zero; an invalid present value is a typed error rather than a silent fallback to the base rate.
fn video_output_cost_per_second(
    record: &PriceRecord,
    video_resolution: Option<&str>,
) -> Result<Option<Decimal>, DurationCostError> {
    if let Some(suffix) = video_resolution.and_then(sanitize_video_resolution) {
        let tier_key = format!("output_cost_per_second_{suffix}");
        if let Some(rate) = numeric_price(record, &tier_key)? {
            return Ok(Some(rate));
        }
    }
    numeric_price(record, "output_cost_per_second")
}

#[derive(Clone, Copy)]
pub(super) enum NoPriceLog {
    Warn,
    Info,
}

pub struct VideoCostRequest<'request> {
    pub model: &'request str,
    pub duration_seconds: Decimal,
    pub custom_llm_provider: Option<&'request str>,
    pub model_info: Option<&'request PriceRecord>,
    pub video_resolution: Option<&'request str>,
}

// Both video calculators share this once a record is resolved: a per-video-second rate wins even when zero, then the resolution/base per-second rate, then zero with a diagnostic.
pub(super) fn resolved_video_cost(
    record: &PriceRecord,
    duration_seconds: Decimal,
    video_resolution: Option<&str>,
    model: &str,
    no_price: NoPriceLog,
) -> Result<Decimal, DurationCostError> {
    if let Some(rate) = numeric_price(record, "output_cost_per_video_per_second")? {
        return rate
            .checked_mul(duration_seconds)
            .ok_or(DurationCostError::Overflow);
    }
    if let Some(rate) = video_output_cost_per_second(record, video_resolution)? {
        return rate
            .checked_mul(duration_seconds)
            .ok_or(DurationCostError::Overflow);
    }
    match no_price {
        NoPriceLog::Warn => tracing::warn!(model, "no cost information found for video model"),
        NoPriceLog::Info => tracing::info!(model, "no cost information found for video model"),
    }
    Ok(Decimal::ZERO)
}

// Mirrors the source's five-candidate search order; base_model_name strips the provider prefix then immediately rebuilds it, so the first two candidates are the same key by construction, same as the source.
fn candidate_keys(model: &str, custom_llm_provider: Option<&str>) -> [Option<String>; 5] {
    let mut base_model_name = model.to_owned();
    let mut without_provider_prefix = None;
    if let Some(provider) = custom_llm_provider {
        let prefix = format!("{provider}/");
        if model.starts_with(&prefix) {
            let stripped = model.replace(&prefix, "");
            without_provider_prefix = Some(stripped.clone());
            base_model_name = format!("{provider}/{stripped}");
        }
    }
    let model_without_provider = model.rsplit('/').next().unwrap_or(model).to_owned();
    let prefixed_model = custom_llm_provider.map(|provider| format!("{provider}/{model}"));
    [
        Some(base_model_name),
        Some(model.to_owned()),
        Some(model_without_provider),
        without_provider_prefix,
        prefixed_model,
    ]
}

fn resolve_raw_record(
    model: &str,
    custom_llm_provider: Option<&str>,
    lookup: &dyn PriceLookup,
) -> Option<PriceRecord> {
    candidate_keys(model, custom_llm_provider)
        .into_iter()
        .flatten()
        .find_map(|key| lookup.raw_cost_map_entry(&key))
}

// Supplied model-info, including an empty map, bypasses the raw catalog search entirely.
pub fn default_video_cost_calculator(
    request: &VideoCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, DurationCostError> {
    let record = match request.model_info {
        Some(supplied) => supplied.clone(),
        None => resolve_raw_record(request.model, request.custom_llm_provider, lookup).ok_or_else(
            || DurationCostError::UnknownModel {
                model: request.model.to_owned(),
            },
        )?,
    };
    resolved_video_cost(
        &record,
        request.duration_seconds,
        request.video_resolution,
        request.model,
        NoPriceLog::Info,
    )
}

mod tests;
