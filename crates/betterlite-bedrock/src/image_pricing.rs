pub mod error;
mod families;

use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use crate::image_models::family::{Family, select_family};
use crate::image_models::params::Params;
use error::ImagePricingError;
use families::{flat_cost, stability1_cost};

// The returned image count and the request options the per-family catalog keys need; deployment price overrides never reach these calculators.
pub struct ImageCostRequest<'request> {
    pub model: &'request str,
    pub image_count: i64,
    pub size: Option<&'request str>,
    pub optional_params: Option<&'request Params>,
}

// Mirror of litellm/llms/bedrock/image_generation/cost_calculator.py: the family selector picks the calculator.
pub fn image_cost(
    request: &ImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImagePricingError> {
    match select_family(Some(request.model)) {
        Family::Titan | Family::Stability3 | Family::Nova => flat_cost(request, lookup),
        Family::Stability1 => stability1_cost(request, lookup),
    }
}

mod tests;
