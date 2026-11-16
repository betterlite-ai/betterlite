use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::fal::stable_diffusion::error::StableDiffusionError;

pub const SIZE_ALIASES: [(&str, &str); 6] = [
    ("1024x1024", "square_hd"),
    ("512x512", "square"),
    ("768x1024", "portrait_4_3"),
    ("576x1024", "portrait_16_9"),
    ("1024x768", "landscape_4_3"),
    ("1024x576", "landscape_16_9"),
];

pub const FALLBACK_SIZE: &str = "landscape_4_3";

pub fn map_image_size(size: &Json) -> Result<Json, StableDiffusionError> {
    let Json::Text(text) = size else {
        return Err(StableDiffusionError::SizeNotAString);
    };
    let requested = text.as_str();
    if let Some((_, alias)) = SIZE_ALIASES
        .iter()
        .find(|(alias_size, _)| *alias_size == requested)
    {
        return Ok(Json::Text((*alias).to_owned()));
    }
    // Python splits on every x, so a -x- string never parses; the shared parser would read it
    if !requested.contains("-x-")
        && let Some(dimensions) = parsed_dimensions(requested)
    {
        return Ok(dimensions);
    }
    Ok(Json::Text(FALLBACK_SIZE.to_owned()))
}

fn parsed_dimensions(text: &str) -> Option<Json> {
    let (width, height) = parse_image_dimensions(text).ok()?;
    let mut fields = IndexMap::new();
    fields.insert("width".to_owned(), Json::Number(Number::from(width)));
    fields.insert("height".to_owned(), Json::Number(Number::from(height)));
    Some(Json::Object(fields))
}
