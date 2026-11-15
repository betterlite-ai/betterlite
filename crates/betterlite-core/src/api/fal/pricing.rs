mod keyed;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::resolve_image_model_info;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_rates::deployment::{DeploymentPricing, deployment_pricing};
use betterlite_models::token_rates::{ModelInfo, TokenRateError};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cost::image_dispatch::options::OptionalParams;

pub const FAL_KEYED_DEFAULT_QUALITY: &str = "high";
const DEFAULT_KEYED_DIMENSIONS: (u64, u64) = (1024, 768);
pub const FAL_TEXT_TO_IMAGE_DEFAULT_SIZE: &str = "1024-x-768";
pub const FAL_PIXELS_PER_MEGAPIXEL: u64 = 1_048_576;
pub const FAL_NAMED_IMAGE_SIZES: [(&str, &str); 6] = [
    ("square_hd", "1024-x-1024"),
    ("square", "512-x-512"),
    ("portrait_4_3", "768-x-1024"),
    ("portrait_16_9", "576-x-1024"),
    ("landscape_4_3", "1024-x-768"),
    ("landscape_16_9", "1024-x-576"),
];
pub const FAL_AI_QUEUE_DEFAULT_BASE: &str = "https://queue.fal.run";

const FAL_AI_PROVIDER: &str = "fal_ai";

#[derive(Debug, Clone, Copy)]
pub enum FalImageResponse<'response> {
    // Each returned image's provider-specific fields; an image without a mapping is None.
    Image(&'response [Option<&'response IndexMap<String, Json>>]),
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct FalCostRequest<'request> {
    pub model: &'request str,
    pub image_response: FalImageResponse<'request>,
    pub optional_params: Option<&'request OptionalParams>,
    pub model_info: Option<&'request ModelInfo>,
}

#[derive(Debug, thiserror::Error)]
pub enum FalPricingError {
    #[error("image_response must be of type ImageResponse")]
    NotImageResponse,
    #[error(transparent)]
    Rates(#[from] TokenRateError),
    #[error(transparent)]
    ImageCost(#[from] ImageCostError),
    #[error("deployment prices do not serialize: {0}")]
    Prices(#[from] serde_json::Error),
    #[error("cost arithmetic overflow")]
    Overflow,
}

#[must_use]
pub fn fal_ai_queue_base(configured: Option<&str>) -> String {
    configured
        .filter(|value| !value.is_empty())
        .unwrap_or(FAL_AI_QUEUE_DEFAULT_BASE)
        .to_owned()
}

pub fn cost_calculator(
    request: &FalCostRequest<'_>,
    catalog: &ModelCatalog,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, FalPricingError> {
    let FalImageResponse::Image(images) = request.image_response else {
        return Err(FalPricingError::NotImageResponse);
    };
    let model = request
        .model
        .strip_prefix("fal_ai/")
        .unwrap_or(request.model);
    let image_count = Decimal::from(images.len());
    let deployment = deployment_pricing(request.model_info)?;
    if let Some(pricing) = &deployment
        && let Some(cost) = pricing.output_image
    {
        return cost
            .checked_mul(image_count)
            .ok_or(FalPricingError::Overflow);
    }
    let resolution_entry = lookup.raw_cost_map_entry(&format!("{FAL_AI_PROVIDER}/{model}"));
    let resolution = request
        .optional_params
        .and_then(|params| params.get("resolution"));
    if let Some(cost) = resolution_cost_per_image(resolution_entry.as_ref(), resolution)? {
        return cost
            .checked_mul(image_count)
            .ok_or(FalPricingError::Overflow);
    }
    let keyed_costs =
        keyed::keyed_costs_for(catalog, lookup, model, request.optional_params, images)?;
    if keyed_costs.iter().all(Option::is_some) {
        return keyed_costs
            .iter()
            .flatten()
            .try_fold(Decimal::ZERO, |total, cost| {
                total.checked_add(*cost).ok_or(FalPricingError::Overflow)
            });
    }
    fallback_cost(deployment.as_ref(), lookup, model, images, &keyed_costs)
}

pub fn fal_ai_passthrough_cost(
    model: &str,
    request_body: &OptionalParams,
    lookup: &dyn PriceLookup,
) -> Result<Option<Decimal>, FalPricingError> {
    let Some(entry) = lookup.raw_cost_map_entry(&format!("{FAL_AI_PROVIDER}/{model}")) else {
        return Ok(None);
    };
    let Some(cost_per_image) = passthrough_cost_per_image(&entry, request_body)? else {
        return Ok(None);
    };
    let count = requested_image_count(request_body);
    cost_per_image
        .checked_mul(Decimal::from(count))
        .map(Some)
        .ok_or(FalPricingError::Overflow)
}

fn fallback_cost(
    deployment: Option<&DeploymentPricing>,
    lookup: &dyn PriceLookup,
    model: &str,
    images: &[Option<&IndexMap<String, Json>>],
    keyed_costs: &[Option<Decimal>],
) -> Result<Decimal, FalPricingError> {
    let deployment_record = deployment
        .map(DeploymentPricing::price_record)
        .transpose()?;
    let resolved = resolve_image_model_info(
        model,
        Some(FAL_AI_PROVIDER),
        deployment_record.as_ref(),
        lookup,
    )?;
    let output_cost_per_image =
        image_price(resolved.get("output_cost_per_image"))?.unwrap_or(Decimal::ZERO);
    let output_cost_per_pixel = image_price(resolved.get("output_cost_per_pixel"))?;
    let mut total = Decimal::ZERO;
    for (image, keyed) in images.iter().zip(keyed_costs) {
        let cost = match keyed {
            Some(cost) => *cost,
            None => flat_cost_per_image(*image, output_cost_per_image, output_cost_per_pixel)?,
        };
        total = total.checked_add(cost).ok_or(FalPricingError::Overflow)?;
    }
    Ok(total)
}

fn image_dimensions(fields: Option<&IndexMap<String, Json>>) -> Option<(u64, u64)> {
    let provider_specific_fields = fields?;
    Some((
        positive_integer(provider_specific_fields.get("width"))?,
        positive_integer(provider_specific_fields.get("height"))?,
    ))
}

fn flat_cost_per_image(
    image: Option<&IndexMap<String, Json>>,
    output_cost_per_image: Decimal,
    output_cost_per_pixel: Option<Decimal>,
) -> Result<Decimal, FalPricingError> {
    let Some(dimensions) = image_dimensions(image) else {
        return Ok(output_cost_per_image);
    };
    let Some(pixel) = output_cost_per_pixel else {
        return Ok(output_cost_per_image);
    };
    let megapixels = (u128::from(dimensions.0).saturating_mul(u128::from(dimensions.1)))
        .div_ceil(u128::from(FAL_PIXELS_PER_MEGAPIXEL));
    let units = u64::try_from(megapixels).map_err(|_| FalPricingError::Overflow)?;
    pixel
        .checked_mul(Decimal::from(FAL_PIXELS_PER_MEGAPIXEL))
        .and_then(|value| value.checked_mul(Decimal::from(units)))
        .ok_or(FalPricingError::Overflow)
}

fn resolution_key(resolution: Option<&Json>) -> Option<String> {
    match resolution? {
        Json::Number(number) => number
            .as_i64()
            .map(|value| value.to_string())
            .or_else(|| number.as_u64().map(|value| value.to_string())),
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn resolution_cost_per_image(
    entry: Option<&PriceRecord>,
    resolution: Option<&Json>,
) -> Result<Option<Decimal>, ImageCostError> {
    let (Some(entry), Some(key)) = (entry, resolution_key(resolution)) else {
        return Ok(None);
    };
    image_price(entry.get(&format!("output_cost_per_image_{key}")))
}

fn requested_image_count(request_body: &OptionalParams) -> u64 {
    match request_body.get("num_images") {
        Some(Json::Number(number)) => number.as_u64().filter(|count| *count > 0).unwrap_or(1),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 1,
    }
}

fn passthrough_cost_per_image(
    entry: &PriceRecord,
    request_body: &OptionalParams,
) -> Result<Option<Decimal>, ImageCostError> {
    if let Some(cost) = resolution_cost_per_image(Some(entry), request_body.get("resolution"))? {
        return Ok(Some(cost));
    }
    image_price(entry.get("output_cost_per_image"))
}

fn image_price(value: Option<&Scalar>) -> Result<Option<Decimal>, ImageCostError> {
    match value {
        Some(Scalar::Bool(flag)) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(ImageCostError::Rate),
        Some(Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_)) | None => {
            Ok(None)
        }
    }
}

fn positive_integer(value: Option<&Json>) -> Option<u64> {
    let Json::Number(number) = value? else {
        return None;
    };
    number.as_u64().filter(|integer| *integer > 0)
}

mod tests;
