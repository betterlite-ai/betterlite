use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::{flat_image_cost, resolve_image_model_info};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use rust_decimal::Decimal;

use crate::cost::image_dispatch::policy::ImageResponseView;

pub const PROVIDER: &str = "recraft";

// LiteLLM's Recraft cost_calculator prices the returned images, and resolves the model before it reads the response type
pub fn cost_calculator(
    model: &str,
    response: ImageResponseView<'_>,
    model_info: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImageCostError> {
    let resolved = resolve_image_model_info(model, Some(PROVIDER), model_info, lookup)?;
    let ImageResponseView::Image(billing) = response else {
        return Err(ImageCostError::NotImageResponse);
    };
    flat_image_cost(Some(&resolved), billing.image_count)
}
