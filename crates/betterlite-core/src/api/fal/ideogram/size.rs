use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use indexmap::IndexMap;
use serde_json::Number;

pub const SIZE_ALIASES: [(&str, &str); 6] = [
    ("1024x1024", "square_hd"),
    ("512x512", "square"),
    ("1024x768", "landscape_4_3"),
    ("768x1024", "portrait_4_3"),
    ("1536x1024", "landscape_16_9"),
    ("1024x1536", "portrait_16_9"),
];

pub const FALLBACK_SIZE: &str = "square_hd";

#[must_use]
pub fn map_image_size(size: &Json) -> Json {
    match size {
        Json::Object(fields) => retained_dimensions(fields).unwrap_or_else(|| size.clone()),
        Json::Text(text) => map_size_text(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => size.clone(),
    }
}

fn retained_dimensions(fields: &IndexMap<String, Json>) -> Option<Json> {
    let (Some(width), Some(height)) = (fields.get("width"), fields.get("height")) else {
        return None;
    };
    (is_integer_value(width) && is_integer_value(height))
        .then(|| dimensions_object(width.clone(), height.clone()))
}

fn is_integer_value(value: &Json) -> bool {
    match value {
        Json::Bool(_) => true,
        Json::Number(number) => !number.to_string().contains(['.', 'e', 'E']),
        Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

fn dimensions_object(width: Json, height: Json) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), width),
        ("height".to_owned(), height),
    ]))
}

fn map_size_text(text: &str) -> Json {
    let normalized = text.trim();
    if let Some((_, alias)) = SIZE_ALIASES.iter().find(|(size, _)| *size == normalized) {
        return Json::Text((*alias).to_owned());
    }
    // Python splits on every x, so a -x- string never parses; the shared parser would read it
    if !normalized.contains("-x-")
        && let Some(dimensions) = parsed_dimensions(normalized)
    {
        return dimensions;
    }
    Json::Text(FALLBACK_SIZE.to_owned())
}

fn parsed_dimensions(text: &str) -> Option<Json> {
    let (width, height) = parse_image_dimensions(text).ok()?;
    Some(dimensions_object(
        Json::Number(Number::from(width)),
        Json::Number(Number::from(height)),
    ))
}
