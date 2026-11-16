use betterlite_logging::langfuse_assembly::usage::cost_value;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use rust_decimal::{Decimal, RoundingStrategy};

const RESOLUTION_TIERS: [(u128, &str); 3] =
    [(600_000, "480p"), (1_500_000, "720p"), (4_000_000, "1080p")];

// _ratio_to_resolution: a colon ratio whose sides are all ASCII digits, compared against the strict area limits
#[must_use]
pub fn ratio_to_resolution(ratio: Option<&Json>) -> Option<&'static str> {
    let Some(Json::Text(text)) = ratio else {
        return None;
    };
    let (width_text, height_text) = text.split_once(':')?;
    if width_text.is_empty()
        || height_text.is_empty()
        || !width_text.bytes().all(|byte| byte.is_ascii_digit())
        || !height_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    // a side of zeros is a zero area, and a side too long for u128 is far past every tier
    let width_digits = width_text.trim_start_matches('0');
    let height_digits = height_text.trim_start_matches('0');
    if width_digits.is_empty() || height_digits.is_empty() {
        return Some("480p");
    }
    let area = match (width_digits.parse::<u128>(), height_digits.parse::<u128>()) {
        (Ok(width_pixels), Ok(height_pixels)) => width_pixels.checked_mul(height_pixels),
        _ => None,
    };
    let resolution = RESOLUTION_TIERS
        .iter()
        .find(|(limit, _)| area.is_some_and(|pixels| pixels < *limit))
        .map_or("4k", |(_, label)| *label);
    Some(resolution)
}

// _estimated_cost_usd: credits are numeric as a JSON number or bool, and 0.01 USD per credit is exact
#[must_use]
pub fn estimated_cost_usd(credits: Option<&Json>) -> Option<Json> {
    let amount = match credits? {
        Json::Bool(flag) => Decimal::from(u8::from(*flag)),
        Json::Number(number) => parse_decimal_lexeme(&number.to_string()).ok()?,
        Json::Text(_) | Json::Null | Json::List(_) | Json::Object(_) => return None,
    };
    let usd = amount.checked_mul(Decimal::new(1, 2))?;
    Some(cost_value(usd))
}

// _progress_percent: half-even rounding of the decimal share, clamped to whole percent
#[must_use]
pub fn progress_percent(progress: f64) -> i64 {
    let scaled =
        Decimal::from_f64(progress).and_then(|value| value.checked_mul(Decimal::ONE_HUNDRED));
    let Some(percent) = scaled else {
        // a share Decimal cannot hold keeps its sign, and a NaN share counts as no progress
        return if progress.is_nan() || progress.is_sign_negative() {
            0
        } else {
            100
        };
    };
    let rounded = percent.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);
    // the clamp leaves a whole number in range, so the conversion always fits
    rounded
        .clamp(Decimal::ZERO, Decimal::ONE_HUNDRED)
        .to_i64()
        .unwrap_or_default()
}
