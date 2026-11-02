use std::borrow::Cow;

use rust_decimal::Decimal;

use super::error::ImageCostError;
use super::pricing::raw_price;
use crate::price_lookup::{PriceLookup, PriceRecord};

const DEFAULT_SIZE: &str = "1024-x-1024";
const FALLBACK_QUALITY: &str = "high";

pub struct DefaultImageCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub quality: Option<&'request str>,
    pub n_images: Option<i64>,
    pub size: Option<&'request str>,
    pub model_info: Option<&'request PriceRecord>,
}

// Searches the eight pinned catalog candidates in order; a match is required unless the caller supplied its own table. Every table (the supplied one before the public one) is then searched key by key for the first present input/output-per-image or per-pixel price, an explicit zero included, and that price wins immediately.
pub fn default_image_cost_calculator(
    request: &DefaultImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    let size = request
        .size
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_SIZE);
    let (height, width) = parse_image_dimensions(size)?;
    let candidates = catalog_candidates(
        request.model,
        request.custom_llm_provider,
        request.quality,
        size,
    );
    let matched = candidates
        .iter()
        .flatten()
        .find_map(|candidate| lookup.raw_cost_map_entry(candidate));

    if matched.is_none() && request.model_info.is_none() {
        return Err(ImageCostError::UnknownModel {
            model: request.model.to_owned(),
        });
    }

    let image_count = request.n_images.unwrap_or(1);
    let pixel_units = height
        .checked_mul(width)
        .and_then(|value| value.checked_mul(image_count))
        .ok_or(ImageCostError::DimensionOverflow)?;
    first_declared_price(
        request.model_info,
        matched.as_ref(),
        image_count,
        pixel_units,
    )?
    .map_or_else(
        || {
            Err(ImageCostError::NoPrice {
                model: request.model.to_owned(),
            })
        },
        Ok,
    )
}

// The supplied table before the public one, each searched key by key for the first present input/output-per-image or per-pixel price; an explicit zero wins immediately.
fn first_declared_price(
    model_info: Option<&PriceRecord>,
    matched: Option<&PriceRecord>,
    image_count: i64,
    pixel_units: i64,
) -> Result<Option<Decimal>, ImageCostError> {
    let unit_counts: [(&str, i64); 3] = [
        ("input_cost_per_image", image_count),
        ("output_cost_per_image", image_count),
        ("input_cost_per_pixel", pixel_units),
    ];
    for table in [model_info, matched].into_iter().flatten() {
        for (key, units) in unit_counts {
            if let Some(price) = raw_price(table, key)? {
                let cost = price
                    .checked_mul(Decimal::from(units))
                    .ok_or(ImageCostError::Overflow)?;
                return Ok(Some(cost));
            }
        }
    }
    Ok(None)
}

// Converts an x separator only when -x- is absent, then requires exactly two Python-int components, preserving zero or negative dimensions rather than inventing a positivity check.
pub fn parse_image_dimensions(size: &str) -> Result<(i64, i64), ImageCostError> {
    let normalized = normalize_size_separator(size);
    let mut parts = normalized.split("-x-");
    let (Some(first), Some(second), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(ImageCostError::MalformedSize {
            size: size.to_owned(),
        });
    };
    Ok((
        parse_signed_int_literal(first)?,
        parse_signed_int_literal(second)?,
    ))
}

fn normalize_size_separator(size: &str) -> Cow<'_, str> {
    if size.contains('x') && !size.contains("-x-") {
        Cow::Owned(size.replace('x', "-x-"))
    } else {
        Cow::Borrowed(size)
    }
}

// Python int() grammar: an optional sign, PEP-515-underscored digits, no decimal point or exponent. Duplicated from pricing's float-grammar helpers since those allow a point and aren't pub.
fn parse_signed_int_literal(text: &str) -> Result<i64, ImageCostError> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    if !is_digit_run(digits) {
        return Err(ImageCostError::MalformedSize {
            size: text.to_owned(),
        });
    }
    let cleaned: String = digits
        .chars()
        .filter(|&character| character != '_')
        .collect();
    let magnitude: i64 = cleaned
        .parse()
        .map_err(|_error| ImageCostError::DimensionOverflow)?;
    if negative {
        magnitude
            .checked_neg()
            .ok_or(ImageCostError::DimensionOverflow)
    } else {
        Ok(magnitude)
    }
}

// PEP 515: a digit run may carry single underscores between two digits, never leading, trailing or doubled.
fn is_digit_run(text: &str) -> bool {
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

// The eight pinned candidates, in search order: quality/base, provider-first-quality, base, high/base, quality/last-segment, last-segment, model, stripped-provider model. All occurrences of provider/ are stripped only once the model starts with it.
fn catalog_candidates(
    model: &str,
    custom_llm_provider: Option<&str>,
    quality: Option<&str>,
    size: &str,
) -> [Option<String>; 8] {
    let provider = custom_llm_provider.filter(|value| !value.is_empty());
    let stripped_model = provider
        .filter(|value| model.starts_with(&format!("{value}/")))
        .map(|value| model.replace(&format!("{value}/"), ""));

    let base_model_name = match (provider, stripped_model.as_deref()) {
        (Some(prefix), Some(stripped)) => format!("{prefix}/{size}/{stripped}"),
        _ => format!("{size}/{model}"),
    };
    let last_segment = model.rsplit('/').next().unwrap_or(model);
    let model_without_provider = format!("{size}/{last_segment}");

    let with_quality =
        |base: &str| quality.map_or_else(|| base.to_owned(), |label| format!("{label}/{base}"));
    let provider_quality_candidate = quality.zip(provider).map(|(label, prefix)| {
        format!(
            "{prefix}/{label}/{size}/{}",
            stripped_model.as_deref().unwrap_or(model)
        )
    });

    [
        Some(with_quality(&base_model_name)),
        provider_quality_candidate,
        Some(base_model_name.clone()),
        Some(format!("{FALLBACK_QUALITY}/{base_model_name}")),
        Some(with_quality(&model_without_provider)),
        Some(model_without_provider),
        Some(model.to_owned()),
        stripped_model,
    ]
}

mod tests;
