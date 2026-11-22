use std::str::FromStr as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::StabilityEditError;
use super::types::{EditOptions, EditValue, EditedImageRequest, ImageInput, text_of};

const FILE_FIELDS: [&str; 3] = ["mask", "init_image", "style_image"];
const INT_FIELDS: [&str; 5] = ["left", "right", "up", "down", "seed"];
const FLOAT_FIELDS: [&str; 8] = [
    "strength",
    "creativity",
    "control_strength",
    "grow_mask",
    "fidelity",
    "composition_fidelity",
    "style_strength",
    "change_strength",
];
const TEXT_FIELDS: [&str; 8] = [
    "negative_prompt",
    "aspect_ratio",
    "output_format",
    "model",
    "mode",
    "style_preset",
    "select_prompt",
    "search_prompt",
];

pub fn transform_image_edit_request(
    model: &str,
    prompt: Option<&str>,
    image: Option<&ImageInput>,
    options: &EditOptions,
) -> Result<EditedImageRequest, StabilityEditError> {
    let mut body = IndexMap::new();
    body.insert("output_format".to_owned(), Json::Text("png".to_owned()));
    if let Some(text) = prompt
        && !text.is_empty()
    {
        body.insert("prompt".to_owned(), Json::Text(text.to_owned()));
    }
    if let Some(source) = image {
        insert_image(&mut body, model, source);
    }
    for (name, value) in options {
        if name.starts_with('_') || matches!(value, EditValue::Json(Json::Null)) {
            continue;
        }
        if let Some(entry) = option_entry(name, value)? {
            body.insert(name.clone(), entry);
        }
    }
    Ok(EditedImageRequest {
        body: Json::Object(body),
        files: ImageFiles::new(),
    })
}

// A style-transfer model names its source image init_image; every other family names it image.
fn insert_image(body: &mut IndexMap<String, Json>, model: &str, image: &ImageInput) {
    let encoded = match image {
        ImageInput::Bytes(content) => BASE64.encode(content),
        ImageInput::Text(text) => text.clone(),
    };
    let field = if model.to_lowercase().contains("style-transfer") {
        "init_image"
    } else {
        "image"
    };
    body.insert(field.to_owned(), Json::Text(encoded));
}

fn option_entry(name: &str, value: &EditValue) -> Result<Option<Json>, StabilityEditError> {
    if FILE_FIELDS.contains(&name) {
        return Ok(Some(Json::Text(file_text(value))));
    }
    if INT_FIELDS.contains(&name) {
        return Ok(Some(numeric_field(name, value, Numeric::Int)?));
    }
    if FLOAT_FIELDS.contains(&name) {
        return Ok(Some(numeric_field(name, value, Numeric::Float)?));
    }
    if TEXT_FIELDS.contains(&name) {
        let text = value.to_json().ok_or_else(|| field_error(name))?;
        return Ok(Some(text));
    }
    Ok(None)
}

// The first item of a nonempty list, as bytes, a string, or Python's str(); an empty list prints as [].
fn file_text(value: &EditValue) -> String {
    let item = match value {
        EditValue::List(items) if items.is_empty() => return "[]".to_owned(),
        EditValue::List(items) => items.first().unwrap_or(value),
        other @ (EditValue::Json(_) | EditValue::Bytes(_)) => other,
    };
    match item {
        EditValue::Bytes(content) => BASE64.encode(content),
        EditValue::Json(Json::Text(text)) => text.clone(),
        EditValue::Json(json) => text_of(json),
        EditValue::List(_) => "object".to_owned(),
    }
}

#[derive(Clone, Copy)]
enum Numeric {
    Int,
    Float,
}

// A ValueError or TypeError keeps the original value; a number the range cannot hold is a conversion error.
fn numeric_field(name: &str, value: &EditValue, kind: Numeric) -> Result<Json, StabilityEditError> {
    match value {
        EditValue::Json(Json::Text(text)) => match kind {
            Numeric::Int => Ok(parse_int_text(text).unwrap_or_else(|| Json::Text(text.clone()))),
            Numeric::Float => underscore_removed(text.trim())
                .and_then(|normalized| normalized.parse::<f64>().ok())
                .map_or_else(
                    || Ok(Json::Text(text.clone())),
                    |parsed| float_number(name, parsed),
                ),
        },
        EditValue::Json(Json::Number(number)) => match kind {
            Numeric::Int => int_number(name, number),
            Numeric::Float => {
                let parsed = number.as_f64().ok_or_else(|| field_error(name))?;
                float_number(name, parsed)
            }
        },
        EditValue::Json(Json::Bool(flag)) => match kind {
            Numeric::Int => Ok(Json::Number(Number::from(u8::from(*flag)))),
            Numeric::Float => float_number(name, f64::from(u8::from(*flag))),
        },
        EditValue::Json(Json::Null | Json::Object(_) | Json::List(_))
        | EditValue::Bytes(_)
        | EditValue::List(_) => value.to_json().ok_or_else(|| field_error(name)),
    }
}

// Python's int() truncates toward zero, and a value the range cannot hold raises.
fn int_number(name: &str, number: &Number) -> Result<Json, StabilityEditError> {
    let lexeme = number.to_string();
    let digits = lexeme.strip_prefix('-').unwrap_or(lexeme.as_str());
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(Json::Number(number.clone()));
    }
    let truncated = number.as_f64().ok_or_else(|| field_error(name))?.trunc();
    if !truncated.is_finite() {
        return Err(field_error(name));
    }
    let rendered = format!("{truncated:.0}");
    let normalized = if rendered == "-0" {
        "0"
    } else {
        rendered.as_str()
    };
    Number::from_str(normalized)
        .map(Json::Number)
        .map_err(|_error| field_error(name))
}

// Python's json writes a whole float with its fraction, and Infinity has no standard JSON form.
fn float_number(name: &str, value: f64) -> Result<Json, StabilityEditError> {
    Number::from_f64(value)
        .map(Json::Number)
        .ok_or_else(|| field_error(name))
}

fn parse_int_text(text: &str) -> Option<Json> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let lexeme = underscore_removed(digits)?;
    if lexeme.is_empty() || !lexeme.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = lexeme.trim_start_matches('0');
    let magnitude = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    let signed = if negative && magnitude != "0" {
        format!("-{magnitude}")
    } else {
        magnitude.to_owned()
    };
    Number::from_str(&signed).ok().map(Json::Number)
}

// PEP 515: an underscore may only sit between two digits, so "1_0.5" reads as 10.5.
fn underscore_removed(text: &str) -> Option<String> {
    let mut normalized = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut previous_digit = false;
    while let Some(character) = characters.next() {
        if character == '_' {
            if !previous_digit || !characters.peek().is_some_and(char::is_ascii_digit) {
                return None;
            }
            previous_digit = false;
            continue;
        }
        normalized.push(character);
        previous_digit = character.is_ascii_digit();
    }
    Some(normalized)
}

fn field_error(name: &str) -> StabilityEditError {
    StabilityEditError::Field {
        field: name.to_owned(),
    }
}
