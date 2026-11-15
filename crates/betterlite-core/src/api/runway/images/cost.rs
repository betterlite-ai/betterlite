use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::{raw_price, resolve_image_model_info};
use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use crate::cost::image_dispatch::policy::{ImageResponseView, PricedImage};

pub const CUSTOM_LLM_PROVIDER: &str = "runwayml";

// LiteLLM resolves the model price before it looks at the response, so an unknown model fails before a wrong response type does.
pub fn cost_calculator(
    image: &PricedImage<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    let resolved =
        resolve_image_model_info(image.model, Some(CUSTOM_LLM_PROVIDER), image.prices, lookup)?;
    let price = raw_price(&resolved, "output_cost_per_image")?.unwrap_or(Decimal::ZERO);
    let ImageResponseView::Image(billing) = image.response else {
        return Err(ImageCostError::NotImageResponse);
    };
    price
        .checked_mul(Decimal::from(billing.image_count))
        .ok_or(ImageCostError::Overflow)
}
