use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::PriceLookup;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{
    DEFAULT_KEYED_DIMENSIONS, FAL_AI_PROVIDER, FAL_KEYED_PRICING_DEFAULT_QUALITY,
    FAL_NAMED_IMAGE_SIZES, FAL_TEXT_TO_IMAGE_DEFAULT_SIZE, FalPricingError, image_dimensions,
    image_price, positive_integer,
};
use crate::cost::image_dispatch::options::OptionalParams;

pub(super) fn keyed_costs_for(
    catalog: &ModelCatalog,
    lookup: &dyn PriceLookup,
    model: &str,
    optional_params: Option<&OptionalParams>,
    images: &[Option<&IndexMap<String, Json>>],
) -> Result<Vec<Option<Decimal>>, FalPricingError> {
    images
        .iter()
        .map(|image| {
            keyed_cost_per_image(catalog, lookup, model, *image, optional_params)
                .map_err(FalPricingError::from)
        })
        .collect()
}

pub(super) fn keyed_size(optional_params: Option<&OptionalParams>) -> Option<String> {
    match optional_params.and_then(|params| params.get("image_size")) {
        None | Some(Json::Null) => Some(FAL_TEXT_TO_IMAGE_DEFAULT_SIZE.to_owned()),
        Some(Json::Text(size)) if size == "auto" => Some(FAL_TEXT_TO_IMAGE_DEFAULT_SIZE.to_owned()),
        Some(Json::Text(size)) => FAL_NAMED_IMAGE_SIZES
            .iter()
            .find(|(name, _)| *name == size.as_str())
            .map(|(_, dimensions)| (*dimensions).to_owned()),
        Some(Json::Object(size)) => {
            let width = positive_integer(size.get("width"))?;
            let height = positive_integer(size.get("height"))?;
            Some(format!("{width}-x-{height}"))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
    }
}

pub(super) fn keyed_quality(optional_params: Option<&OptionalParams>) -> &str {
    match optional_params.and_then(|params| params.get("quality")) {
        Some(Json::Text(quality)) if quality != "auto" => quality,
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => FAL_KEYED_PRICING_DEFAULT_QUALITY,
    }
}

pub(super) fn parse_keyed_dimensions(size: Option<&str>) -> Option<(u64, u64)> {
    let (width, height) = size?.split_once("-x-")?;
    if height.contains("-x-") {
        return None;
    }
    let parsed_width = width.parse::<u64>().ok().filter(|value| *value > 0)?;
    let parsed_height = height.parse::<u64>().ok().filter(|value| *value > 0)?;
    Some((parsed_width, parsed_height))
}

struct KeyedRow {
    width: u64,
    height: u64,
    price: Decimal,
}

fn keyed_rows(
    catalog: &ModelCatalog,
    lookup: &dyn PriceLookup,
    model: &str,
    quality: &str,
) -> Result<Vec<KeyedRow>, ImageCostError> {
    let prefix = format!("{FAL_AI_PROVIDER}/{quality}/");
    let suffix = format!("/{model}");
    let mut rows = Vec::new();
    for key in catalog.entries.keys() {
        let Some(remainder) = key.strip_prefix(&prefix) else {
            continue;
        };
        let Some(size) = remainder.strip_suffix(&suffix) else {
            continue;
        };
        let Some(dimensions) = parse_keyed_dimensions(Some(size)) else {
            continue;
        };
        let Some(entry) = lookup.raw_cost_map_entry(key) else {
            continue;
        };
        let Some(price) = image_price(entry.get("output_cost_per_image"))? else {
            continue;
        };
        rows.push(KeyedRow {
            width: dimensions.0,
            height: dimensions.1,
            price,
        });
    }
    Ok(rows)
}

fn keyed_cost_per_image(
    catalog: &ModelCatalog,
    lookup: &dyn PriceLookup,
    model: &str,
    image: Option<&IndexMap<String, Json>>,
    optional_params: Option<&OptionalParams>,
) -> Result<Option<Decimal>, ImageCostError> {
    let quality = keyed_quality(optional_params);
    let rows = keyed_rows(catalog, lookup, model, quality)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let target = image_dimensions(image)
        .or_else(|| parse_keyed_dimensions(keyed_size(optional_params).as_deref()))
        .unwrap_or(DEFAULT_KEYED_DIMENSIONS);
    let target_pixels = u128::from(target.0).saturating_mul(u128::from(target.1));
    let best = rows.iter().min_by_key(|row| {
        let area = u128::from(row.width).saturating_mul(u128::from(row.height));
        (area.abs_diff(target_pixels), area)
    });
    Ok(best.map(|row| row.price))
}
