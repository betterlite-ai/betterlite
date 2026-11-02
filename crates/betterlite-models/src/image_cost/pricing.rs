use rust_decimal::Decimal;

use super::error::ImageCostError;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::{self, Scalar};

const IMAGE_TOKEN_RATE_KEYS: [&str; 4] = [
    "input_cost_per_token",
    "output_cost_per_token",
    "input_cost_per_image_token",
    "output_cost_per_image_token",
];

// Any nonnull token rate, including an explicit zero, counts as declared.
#[must_use]
pub fn prices_tokens(model_info: &PriceRecord) -> bool {
    IMAGE_TOKEN_RATE_KEYS
        .iter()
        .any(|key| !matches!(model_info.get(*key), None | Some(Scalar::Null)))
}

pub fn flat_image_cost(
    model_info: Option<&PriceRecord>,
    image_count: i64,
) -> Result<Decimal, ImageCostError> {
    let Some(info) = model_info else {
        return Ok(Decimal::ZERO);
    };
    let per_image = tolerant_rate(info, "output_cost_per_image")?.unwrap_or(Decimal::ZERO);
    per_image
        .checked_mul(Decimal::from(image_count))
        .ok_or(ImageCostError::Overflow)
}

// A soft lookup with no supplied table propagates the lookup's own failure; a supplied table overlays the public entry's fields, or stands alone when the public lookup finds nothing.
pub fn resolve_image_model_info(
    model: &str,
    custom_llm_provider: Option<&str>,
    model_info: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Result<PriceRecord, ImageCostError> {
    let found = lookup.effective_model_info(model, custom_llm_provider);
    match (model_info, found) {
        (None, None) => Err(ImageCostError::UnknownModel {
            model: model.to_owned(),
        }),
        (None, Some(shared)) => Ok(shared),
        (Some(supplied), None) => Ok(supplied.clone()),
        (Some(supplied), Some(mut shared)) => {
            shared.extend(supplied.clone());
            Ok(shared)
        }
    }
}

// Azure's pixel rate reads the already-resolved table's own tolerant rate first, then its matched key's public raw entry; a malformed string there is absent, not an explicit zero, so it still falls through to the catalog rather than winning early.
pub fn input_cost_per_pixel(
    resolved: &PriceRecord,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    if let Some(value) = tolerant_rate(resolved, "input_cost_per_pixel")? {
        return Ok(value);
    }
    let Some(Scalar::Text(key)) = resolved.get("key") else {
        return Ok(Decimal::ZERO);
    };
    let Some(entry) = lookup.raw_cost_map_entry(key) else {
        return Ok(Decimal::ZERO);
    };
    Ok(raw_price(&entry, "input_cost_per_pixel")?.unwrap_or(Decimal::ZERO))
}

// A declared `float | None` field read without any string tolerance: any other JSON shape names the field as a type mismatch instead of being coerced.
pub fn raw_price(record: &PriceRecord, key: &str) -> Result<Option<Decimal>, ImageCostError> {
    let Some(scalar) = record.get(key) else {
        return Ok(None);
    };
    if matches!(scalar, Scalar::Null) {
        return Ok(None);
    }
    let Scalar::Number(number) = scalar else {
        return Err(ImageCostError::WrongFieldType {
            field: key.to_owned(),
        });
    };
    Ok(Some(parse_decimal_lexeme(&number.to_string())?))
}

// Mirrors _get_cost_per_unit's default_value=None path: a malformed string, an array or an object is absent, letting the caller's own fallback apply instead of becoming a hard zero.
fn tolerant_rate(model_info: &PriceRecord, key: &str) -> Result<Option<Decimal>, ImageCostError> {
    match model_info.get(key) {
        None | Some(Scalar::Null | Scalar::Array(_) | Scalar::Object(_)) => Ok(None),
        Some(scalar @ (Scalar::Bool(_) | Scalar::Number(_))) => {
            Ok(Some(tiered_pricing::coerce_cost_per_token(Some(scalar))?))
        }
        Some(scalar @ Scalar::Text(text)) => {
            if matches_float_literal_grammar(text.trim()) {
                Ok(Some(tiered_pricing::coerce_cost_per_token(Some(scalar))?))
            } else {
                Ok(None)
            }
        }
    }
}

// Python float() grammar, duplicated from tiered_pricing's private check since it isn't pub.
fn matches_float_literal_grammar(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.eq_ignore_ascii_case("inf")
        || body.eq_ignore_ascii_case("infinity")
        || body.eq_ignore_ascii_case("nan")
    {
        return true;
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    is_decimal_mantissa(mantissa)
        && exponent.is_none_or(|digits| {
            is_underscored_digits(digits.strip_prefix(['+', '-']).unwrap_or(digits))
        })
}

fn is_decimal_mantissa(text: &str) -> bool {
    let Some((int_part, frac_part)) = text.split_once('.') else {
        return is_underscored_digits(text);
    };
    (!int_part.is_empty() || !frac_part.is_empty())
        && (int_part.is_empty() || is_underscored_digits(int_part))
        && (frac_part.is_empty() || is_underscored_digits(frac_part))
}

// PEP 515: a digit run may carry single underscores between two digits, never leading, trailing or doubled. Shared with default_calculator's int-literal grammar, which has the same rule.
pub(super) fn is_underscored_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if first == b'_' || last == b'_' {
        return false;
    }
    let mut previous_was_underscore = false;
    for &current in bytes {
        match current {
            b'_' if previous_was_underscore => return false,
            b'_' => previous_was_underscore = true,
            digit if digit.is_ascii_digit() => previous_was_underscore = false,
            _ => return false,
        }
    }
    true
}

mod tests;
