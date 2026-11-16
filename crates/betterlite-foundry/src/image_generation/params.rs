use betterlite_models::decisions::Json;
use betterlite_models::model_registration::merge::convert_stringified_numbers;
use betterlite_openai::image_generation::family::{
    ImageFamily, map_openai_params as map_gpt_params,
};
use indexmap::IndexMap;
use serde_json::Number;

use super::error::FoundryImageError;
use super::flux::{FLUX2_DROPPED_OPENAI_PARAMS, FLUX2_OPENAI_PARAMS, is_flux2_model};

pub type ImageParams = IndexMap<String, Json>;

pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: &ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FoundryImageError> {
    if !is_flux2_model(model) {
        let mut mapped = optional_params.clone();
        map_gpt_params(
            ImageFamily::GptImage,
            model,
            non_default_params,
            &mut mapped,
            drop_params,
        )?;
        return Ok(mapped);
    }
    let mut unsupported: Vec<String> = Vec::new();
    for name in non_default_params.keys() {
        if !FLUX2_OPENAI_PARAMS.contains(&name.as_str()) {
            unsupported.push(name.clone());
        }
    }
    if !unsupported.is_empty() && !drop_params {
        return Err(FoundryImageError::UnsupportedParams {
            model: model.to_owned(),
            names: unsupported,
            supported: &FLUX2_OPENAI_PARAMS,
        });
    }
    let mut mapped = ImageParams::new();
    for (name, value) in non_default_params {
        if FLUX2_OPENAI_PARAMS.contains(&name.as_str()) {
            map_flux2_param(name, value, &mut mapped)?;
        }
    }
    let mut result = optional_params.clone();
    result.extend(mapped);
    Ok(result)
}

fn map_flux2_param(
    name: &str,
    value: &Json,
    mapped: &mut ImageParams,
) -> Result<(), FoundryImageError> {
    if FLUX2_DROPPED_OPENAI_PARAMS.contains(&name) {
        return Ok(());
    }
    if name == "size" {
        return map_size_param(value, mapped);
    }
    if name == "n" {
        return map_named_param(value, "num_images", mapped);
    }
    map_named_param(value, name, mapped)
}

// a text value is converted the way LiteLLM calls int() or float() on it, and every other JSON value is passed as sent
fn map_named_param(
    value: &Json,
    target: &str,
    mapped: &mut ImageParams,
) -> Result<(), FoundryImageError> {
    if let Json::Text(text) = value {
        if matches!(
            target,
            "num_images" | "width" | "height" | "steps" | "seed" | "safety_tolerance"
        ) {
            let parsed = whole_number(text).ok_or_else(|| FoundryImageError::WholeNumber {
                field: target.to_owned(),
                value: text.clone(),
            })?;
            mapped.insert(target.to_owned(), Json::Number(parsed));
            return Ok(());
        }
        if target == "guidance" {
            let parsed = decimal_number(text).ok_or_else(|| FoundryImageError::Decimal {
                field: target.to_owned(),
                value: text.clone(),
            })?;
            mapped.insert(target.to_owned(), Json::Number(parsed));
            return Ok(());
        }
    }
    mapped.insert(target.to_owned(), value.clone());
    Ok(())
}

fn map_size_param(value: &Json, mapped: &mut ImageParams) -> Result<(), FoundryImageError> {
    let Json::Text(text) = value else {
        return Err(FoundryImageError::InvalidSize {
            size: value_text(value),
        });
    };
    let lowered = text.to_lowercase();
    if lowered == "auto" {
        return Ok(());
    }
    let (width, height) = size_dimensions(&lowered)
        .ok_or_else(|| FoundryImageError::InvalidSize { size: text.clone() })?;
    mapped.insert("width".to_owned(), Json::Number(width));
    mapped.insert("height".to_owned(), Json::Number(height));
    Ok(())
}

// str::split yields exactly two parts for WidthxHeight and three or more for a value with a second x
fn size_dimensions(size: &str) -> Option<(Number, Number)> {
    let mut parts = size.split('x');
    let width = parts.next()?;
    let height = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((whole_number(width)?, whole_number(height)?))
}

// betterlite-models builds the number from the int() text cleaned below and keeps every digit
fn whole_number(text: &str) -> Option<Number> {
    let cleaned = integer_text(text)?;
    match convert_stringified_numbers(&cleaned) {
        Json::Number(number) => Some(number),
        Json::Text(_) | Json::Bool(_) | Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's int() takes surrounding whitespace, one sign and digit-grouping underscores
fn integer_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let unsigned = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    let digits = without_underscores(unsigned)?;
    if digits.is_empty() {
        return None;
    }
    if digits.bytes().any(|byte| !byte.is_ascii_digit()) {
        return None;
    }
    let sign = if negative { "-" } else { "" };
    Some(format!("{sign}{digits}"))
}

// Python's float() also takes surrounding whitespace and grouping underscores
fn decimal_number(text: &str) -> Option<Number> {
    let cleaned = without_underscores(text.trim())?;
    let parsed = cleaned.parse::<f64>().ok()?;
    Number::from_f64(parsed)
}

// Python allows an underscore only between two digits
fn without_underscores(text: &str) -> Option<String> {
    // no shared cleaner fits: betterlite-models' is private and accepts 1_.5, betterlite-bedrock's is i64-only
    let mut cleaned = String::with_capacity(text.len());
    let mut previous_was_digit = false;
    let mut after_underscore = false;
    for character in text.chars() {
        if character == '_' {
            if !previous_was_digit {
                return None;
            }
            after_underscore = true;
            previous_was_digit = false;
            continue;
        }
        if after_underscore && !character.is_ascii_digit() {
            return None;
        }
        cleaned.push(character);
        after_underscore = false;
        previous_was_digit = character.is_ascii_digit();
    }
    if after_underscore {
        return None;
    }
    Some(cleaned)
}

fn value_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Null => String::from("null"),
        Json::List(items) => format!("{items:?}"),
        Json::Object(fields) => format!("{fields:?}"),
    }
}
