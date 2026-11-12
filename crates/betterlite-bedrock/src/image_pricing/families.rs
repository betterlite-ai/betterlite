use betterlite_models::image_cost::pricing::raw_price;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use rust_decimal::Decimal;

use crate::image_models::params::Field;

use super::ImageCostRequest;
use super::error::ImagePricingError;

const DEFAULT_SIZE: &str = "1024-x-1024";
const DEFAULT_STEPS: i64 = 50;
const STEP_BUCKET_DEFAULT: &str = "50-steps";
const STEP_BUCKET_MAX: &str = "max-steps";

// Titan, Stability 3 and Nova resolve their unqualified model before charging its flat per-image price.
pub(super) fn flat_cost(
    request: &ImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImagePricingError> {
    let entry = lookup.raw_cost_map_entry(request.model).ok_or_else(|| {
        ImagePricingError::UnknownModel {
            model: request.model.to_owned(),
        }
    })?;
    flat_charge(&entry, request.image_count)
}

// Stability 1 keeps the supplied size spelling and folds the step bucket into the catalog key.
pub(super) fn stability1_cost(
    request: &ImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, ImagePricingError> {
    let steps = steps_bucket(
        request
            .optional_params
            .and_then(|params| params.get("steps")),
    )?;
    let size = request
        .size
        .filter(|size| !size.is_empty())
        .unwrap_or(DEFAULT_SIZE);
    let key = format!("{size}/{steps}/{}", request.model);
    let entry = catalog_entry(lookup, &key)?;
    flat_charge(&entry, request.image_count)
}

// A failed lookup errors before the returned image count can short-circuit, matching the source's lookup-before-empty-data order.
fn catalog_entry(lookup: &dyn PriceLookup, key: &str) -> Result<PriceRecord, ImagePricingError> {
    lookup
        .raw_cost_map_entry(key)
        .ok_or_else(|| ImagePricingError::UnknownModel {
            model: key.to_owned(),
        })
}

// A present steps value must compare with fifty exactly like the source's int comparison does.
pub(super) fn steps_bucket(steps: Option<&Field>) -> Result<&'static str, ImagePricingError> {
    match steps {
        None | Some(Field::Bool(_)) => Ok(STEP_BUCKET_DEFAULT),
        Some(Field::Int(value)) => Ok(if *value > DEFAULT_STEPS {
            STEP_BUCKET_MAX
        } else {
            STEP_BUCKET_DEFAULT
        }),
        Some(Field::Float(value)) => Ok(if *value > 50.0 {
            STEP_BUCKET_MAX
        } else {
            STEP_BUCKET_DEFAULT
        }),
        Some(Field::Null | Field::Text(_) | Field::List(_) | Field::Mapping(_)) => {
            Err(ImagePricingError::InvalidSteps)
        }
    }
}

fn flat_charge(entry: &PriceRecord, image_count: i64) -> Result<Decimal, ImagePricingError> {
    raw_price(entry, "output_cost_per_image")?
        .unwrap_or(Decimal::ZERO)
        .checked_mul(Decimal::from(image_count))
        .ok_or(ImagePricingError::Overflow)
}
