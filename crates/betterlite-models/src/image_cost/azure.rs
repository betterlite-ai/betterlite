use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use super::default_calculator::{DefaultImageCostRequest, default_image_cost_calculator};
use super::error::ImageCostError;
use super::pricing::{input_cost_per_pixel, raw_price, resolve_image_model_info};
use super::usage::{ImageUsageCostRequest, TokenDetailCandidate, image_usage_cost};
use super::{ImageBilling, ImageUsageView};
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;

const AZURE_AI_PROVIDER: &str = "azure_ai";

pub enum AzureImageResponse<'request> {
    Image(&'request ImageBilling),
    Other,
}

pub struct AzureImageCostRequest<'request> {
    pub model: &'request str,
    pub model_info: Option<&'request PriceRecord>,
    pub size: Option<&'request str>,
    pub n_images: Option<i64>,
    pub width: Option<&'request RawValue>,
    pub height: Option<&'request RawValue>,
    pub response: AzureImageResponse<'request>,
    pub moment: DateTime<Utc>,
}

// Azure always resolves pricing against its own provider, tries usage first, then a nonzero output_cost_per_image times the explicit count or the returned count, then a nonzero pixel price delegated to the default calculator with the original, unmerged deployment table.
pub fn cost_calculator(
    request: &AzureImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    let resolved = resolve_image_model_info(
        request.model,
        Some(AZURE_AI_PROVIDER),
        request.model_info,
        lookup,
    )?;
    let AzureImageResponse::Image(billing) = &request.response else {
        return Err(ImageCostError::NotImageResponse);
    };

    if let Some(ImageUsageView::Image(details)) = &billing.usage {
        let token_based = image_usage_cost(
            &ImageUsageCostRequest {
                model: request.model,
                custom_llm_provider: Some(AZURE_AI_PROVIDER),
                usage: details,
                model_info: Some(&resolved),
                moment: request.moment,
            },
            lookup,
            database,
        )?;
        if let Some(cost) = token_based {
            return Ok(cost);
        }
    }

    flat_or_pixel_cost(request, &resolved, billing, lookup)
}

// A nonzero output_cost_per_image times the explicit count or the returned count wins outright; otherwise a nonzero pixel price delegates to the default calculator with the original, unmerged deployment table.
fn flat_or_pixel_cost(
    request: &AzureImageCostRequest<'_>,
    resolved: &PriceRecord,
    billing: &ImageBilling,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    let image_count = request.n_images.unwrap_or(billing.image_count);
    if let Some(price) = nonzero_price(resolved, "output_cost_per_image")? {
        return price
            .checked_mul(Decimal::from(image_count))
            .ok_or(ImageCostError::Overflow);
    }
    if input_cost_per_pixel(resolved, lookup)?.is_zero() {
        return Ok(Decimal::ZERO);
    }

    let size = pixel_size(request.width, request.height, request.size, billing)?;
    default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: catalog_key(resolved).unwrap_or(request.model),
            custom_llm_provider: Some(AZURE_AI_PROVIDER),
            quality: None,
            n_images: Some(image_count),
            size: size.as_deref(),
            model_info: request.model_info,
        },
        lookup,
    )
}

// A raw declared float field read with Python truthiness: an explicit zero falls through to pixel pricing rather than winning, unlike the default calculator's own strict presence check.
fn nonzero_price(record: &PriceRecord, key: &str) -> Result<Option<Decimal>, ImageCostError> {
    match raw_price(record, key)? {
        Some(value) if !value.is_zero() => Ok(Some(value)),
        _ => Ok(None),
    }
}

fn catalog_key(resolved: &PriceRecord) -> Option<&str> {
    match resolved.get("key") {
        Some(Scalar::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

// A positive exact-integer width and height override the requested size; a bool does not, since Python's `type(width) is int` excludes bool despite bool being an int subclass.
fn pixel_size(
    width: Option<&RawValue>,
    height: Option<&RawValue>,
    size: Option<&str>,
    billing: &ImageBilling,
) -> Result<Option<String>, ImageCostError> {
    if let (Some(exact_width), Some(exact_height)) =
        (exact_positive_int(width)?, exact_positive_int(height)?)
    {
        return Ok(Some(format!("{exact_width}x{exact_height}")));
    }
    Ok(size.map(ToOwned::to_owned).or_else(|| billing.size.clone()))
}

// LiteLLM's bare `width > 0` accepts any positive Python int; an i64-sized candidate that doesn't fit is a priced dimension, not an absent one, so it is a typed overflow error rather than a silent fallback to size.
fn exact_positive_int(raw_value: Option<&RawValue>) -> Result<Option<i64>, ImageCostError> {
    let Some(raw) = raw_value else {
        return Ok(None);
    };
    let candidate = serde_json::from_str::<TokenDetailCandidate>(raw.get())
        .unwrap_or_else(|_error| TokenDetailCandidate::Other(Box::new(IgnoredAny)));
    let number = match candidate {
        TokenDetailCandidate::Number(number) => number,
        TokenDetailCandidate::Bool(_) | TokenDetailCandidate::Other(_) => {
            return Ok(None);
        }
    };
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return Ok(None);
    }
    match number.as_i64() {
        Some(magnitude) if magnitude > 0 => Ok(Some(magnitude)),
        Some(_) => Ok(None),
        None if text.starts_with('-') => Ok(None),
        None => Err(ImageCostError::DimensionOverflow),
    }
}

mod tests;
