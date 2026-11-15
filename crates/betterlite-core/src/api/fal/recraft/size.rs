use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::fal::images::error::FalImageError;

pub const SQUARE_HD: &str = "square_hd";

const SIZE_PRESETS: [(&str, &str); 6] = [
    ("1024x1024", "square_hd"),
    ("512x512", "square"),
    ("768x1024", "portrait_4_3"),
    ("576x1024", "portrait_16_9"),
    ("1024x768", "landscape_4_3"),
    ("1024x576", "landscape_16_9"),
];

pub fn translate_size(size: &Json) -> Result<(&'static str, Json), FalImageError> {
    let Json::Text(text) = size else {
        return Err(FalImageError::SizeNotText);
    };
    Ok(("image_size", map_image_size(text)))
}

#[must_use]
pub fn map_image_size(size: &str) -> Json {
    if let Some((_, preset)) = SIZE_PRESETS.iter().find(|(alias, _)| *alias == size) {
        return Json::Text((*preset).to_owned());
    }
    custom_dimensions(size).map_or_else(|| Json::Text(SQUARE_HD.to_owned()), Json::Object)
}

// Python splits on a bare x, so Fal's -x- separator reads as one malformed part there and falls to the default
fn custom_dimensions(size: &str) -> Option<IndexMap<String, Json>> {
    if size.contains("-x-") {
        return None;
    }
    let (width, height) = parse_image_dimensions(size).ok()?;
    let mut dimensions = IndexMap::new();
    dimensions.insert("width".to_owned(), Json::Number(Number::from(width)));
    dimensions.insert("height".to_owned(), Json::Number(Number::from(height)));
    Some(dimensions)
}
