use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::error::MaiImageError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["n", "size"];
pub const DEFAULT_WIDTH: i64 = 1024;
pub const DEFAULT_HEIGHT: i64 = 1024;
const MAX_IMAGES_PER_REQUEST: i128 = 1;
const MIN_DIMENSION_PX: i64 = 768;
const MAX_TOTAL_PX: i128 = 1_056_768;
const NAMED_SIZES: [(&str, (i64, i64)); 5] = [
    ("1024x1024", (1024, 1024)),
    ("1792x1024", (1792, 1024)),
    ("1024x1792", (1024, 1792)),
    ("512x512", (512, 512)),
    ("256x256", (256, 256)),
];

// Keys already in optional_params win, size is always dropped at the end and n is never defaulted in
pub fn map_openai_params(
    non_default_params: &Object,
    mut optional_params: Object,
    model: &str,
    drop_params: bool,
) -> Result<Object, MaiImageError> {
    for (name, value) in non_default_params {
        if !optional_params.contains_key(name) {
            map_param(name, value, &mut optional_params, model, drop_params)?;
        }
    }
    optional_params
        .entry("width".to_owned())
        .or_insert_with(|| Json::Number(Number::from(DEFAULT_WIDTH)));
    optional_params
        .entry("height".to_owned())
        .or_insert_with(|| Json::Number(Number::from(DEFAULT_HEIGHT)));
    optional_params.shift_remove("size");
    Ok(optional_params)
}

fn map_param(
    name: &str,
    value: &Json,
    optional_params: &mut Object,
    model: &str,
    drop_params: bool,
) -> Result<(), MaiImageError> {
    match (name, value) {
        ("size", Json::Text(size)) if !size.is_empty() => {
            map_size_param(size, optional_params, model)?;
        }
        ("size", Json::Null | Json::Text(_)) => {}
        ("size", _) => return Err(MaiImageError::FieldType { field: "size" }),
        ("n", Json::Null) => {
            optional_params.insert(name.to_owned(), value.clone());
        }
        ("n", _) => map_count(value, optional_params, model, drop_params)?,
        ("width" | "height", _) => {
            optional_params.insert(name.to_owned(), value.clone());
        }
        _ if drop_params => {}
        _ => {
            return Err(MaiImageError::UnsupportedParam {
                model: model.to_owned(),
                name: name.to_owned(),
            });
        }
    }
    Ok(())
}

fn map_count(
    count: &Json,
    optional_params: &mut Object,
    model: &str,
    drop_params: bool,
) -> Result<(), MaiImageError> {
    if image_count(count, model)? == MAX_IMAGES_PER_REQUEST {
        optional_params.insert("n".to_owned(), count.clone());
        Ok(())
    } else if drop_params {
        Ok(())
    } else {
        Err(MaiImageError::CountNotOne {
            model: model.to_owned(),
            count: describe(count),
        })
    }
}

// A bool is an int in LiteLLM and any other value goes through int(str(n))
fn image_count(count: &Json, model: &str) -> Result<i128, MaiImageError> {
    let parsed = match count {
        Json::Bool(flag) => Some(i128::from(*flag)),
        Json::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        Json::Text(text) => text.trim().parse::<i128>().ok(),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    };
    parsed.ok_or_else(|| MaiImageError::CountNotWhole {
        model: model.to_owned(),
        count: describe(count),
    })
}

fn map_size_param(
    size: &str,
    optional_params: &mut Object,
    model: &str,
) -> Result<(), MaiImageError> {
    let named = NAMED_SIZES.iter().find(|(name, _)| *name == size);
    let (width, height) = if let Some((_, dimensions)) = named {
        *dimensions
    } else if size.contains('x') {
        custom_dimensions(size, model)?
    } else {
        return Err(MaiImageError::UnsupportedSize {
            model: model.to_owned(),
            size: size.to_owned(),
        });
    };
    validate_dimensions(model, size, width, height)?;
    optional_params.insert("width".to_owned(), Json::Number(Number::from(width)));
    optional_params.insert("height".to_owned(), Json::Number(Number::from(height)));
    Ok(())
}

// LiteLLM lowercases first, which changes nothing: an uppercase X next to the required lowercase x never parses
fn custom_dimensions(size: &str, model: &str) -> Result<(i64, i64), MaiImageError> {
    let mut parts = size.split('x');
    let parsed = match (parts.next(), parts.next(), parts.next()) {
        (Some(width), Some(height), None) => width
            .trim()
            .parse::<i64>()
            .ok()
            .zip(height.trim().parse::<i64>().ok()),
        _ => None,
    };
    parsed.ok_or_else(|| MaiImageError::InvalidSize {
        model: model.to_owned(),
        size: size.to_owned(),
    })
}

fn validate_dimensions(
    model: &str,
    size: &str,
    width: i64,
    height: i64,
) -> Result<(), MaiImageError> {
    if width < MIN_DIMENSION_PX || height < MIN_DIMENSION_PX {
        return Err(MaiImageError::SizeBelowMinimum {
            model: model.to_owned(),
            size: size.to_owned(),
        });
    }
    let pixels = i128::from(width).saturating_mul(i128::from(height));
    if pixels > MAX_TOTAL_PX {
        return Err(MaiImageError::SizeAboveMaximum {
            model: model.to_owned(),
            size: size.to_owned(),
            width,
            height,
            pixels,
        });
    }
    Ok(())
}

// The message shows the count as sent, with text quoted
fn describe(count: &Json) -> String {
    match count {
        Json::Null => "null".to_owned(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => format!("\"{text}\""),
        Json::List(_) => "a list".to_owned(),
        Json::Object(_) => "an object".to_owned(),
    }
}
