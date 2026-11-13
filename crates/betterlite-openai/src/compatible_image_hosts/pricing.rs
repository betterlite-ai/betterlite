use betterlite_config::sections::compatible_image_credentials::Host;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::{raw_price, resolve_image_model_info};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use rust_decimal::Decimal;

use super::host_provider;
use crate::api::image_contracts::records::ImageResponse;

pub enum ImageBilling<'response> {
    Response(&'response ImageResponse),
    Other,
}

// the resolver is injected with the host's own provider identity, and only a returned data entry is billed
pub fn compatible_image_cost(
    host: Host,
    model: &str,
    billing: ImageBilling<'_>,
    model_info: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    let resolved = resolve_image_model_info(model, Some(host_provider(host)), model_info, lookup)?;
    let ImageBilling::Response(response) = billing else {
        return Err(ImageCostError::NotImageResponse);
    };
    let image_count = response.data.as_ref().map_or(0, Vec::len);
    let count = i64::try_from(image_count).map_err(|_error| ImageCostError::CounterOverflow)?;
    let per_image = raw_price(&resolved, "output_cost_per_image")?.unwrap_or(Decimal::ZERO);
    per_image
        .checked_mul(Decimal::from(count))
        .ok_or(ImageCostError::Overflow)
}
