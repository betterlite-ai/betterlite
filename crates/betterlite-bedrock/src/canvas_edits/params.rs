use betterlite_models::decisions::Json;
use serde_json::Number;

use super::types::{CanvasEditOptions, CanvasEditValue};
use crate::image_models::sizing::parse_python_int;
use crate::stability_edits::types::text_of;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "n",
    "size",
    "response_format",
    "mask",
    "negativeText",
    "similarityStrength",
    "cfgScale",
    "seed",
    "quality",
    "taskType",
    "maskPrompt",
    "outPaintingMode",
    "imageGenerationConfig",
];

// Keys the transform consumes itself, so drop_params keeps them even though OpenAI does not name them.
const KEPT_PARAMS: [&str; 4] = ["width", "height", "numberOfImages", "mask"];
const PREMIUM: &str = "premium";
const B64_JSON: &str = "b64_json";

pub struct MappedOptions {
    pub options: CanvasEditOptions,
    pub ignored_response_format: Option<IgnoredResponseFormat>,
}

pub struct IgnoredResponseFormat(CanvasEditValue);

impl IgnoredResponseFormat {
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "Nova Canvas image edit ignores response_format={} and returns base64 images",
            ignored_text(&self.0)
        )
    }
}

// LiteLLM logs this and drops the key: Nova Canvas returns base64 images only.
#[must_use]
pub fn map_openai_params(options: &CanvasEditOptions, drop_params: bool) -> MappedOptions {
    let mut mapped = options.clone();
    apply_size_fallback(&mut mapped);
    move_number_of_images(&mut mapped);
    map_quality(&mut mapped);
    let ignored_response_format = take_response_format(&mut mapped);
    if drop_params {
        drop_unknown(&mut mapped);
    }
    MappedOptions {
        options: mapped,
        ignored_response_format,
    }
}

// None when the absent name, or an explicit null, stands for Python's None
pub(super) fn take_value(options: &mut CanvasEditOptions, name: &str) -> Option<CanvasEditValue> {
    match options.shift_remove(name) {
        None | Some(CanvasEditValue::Json(Json::Null)) => None,
        Some(value) => Some(value),
    }
}

// A size that is not "<int>x<int>" leaves width and height unset and does not raise.
pub(super) fn size_pair(value: &CanvasEditValue) -> Option<(Json, Json)> {
    let CanvasEditValue::Json(Json::Text(size)) = value else {
        return None;
    };
    let (width, height) = size.split_once('x')?;
    Some((int_json(width)?, int_json(height)?))
}

fn apply_size_fallback(options: &mut CanvasEditOptions) {
    let Some(size) = options.shift_remove("size") else {
        return;
    };
    let Some((width, height)) = size_pair(&size) else {
        return;
    };
    options.insert("width".to_owned(), CanvasEditValue::Json(width));
    options.insert("height".to_owned(), CanvasEditValue::Json(height));
}

fn move_number_of_images(options: &mut CanvasEditOptions) {
    if let Some(value) = take_value(options, "n") {
        options.insert("numberOfImages".to_owned(), value);
    }
}

// hd and premium both mean premium; every other value is kept as given for the API to judge.
fn map_quality(options: &mut CanvasEditOptions) {
    let Some(value) = take_value(options, "quality") else {
        return;
    };
    let quality = match &value {
        CanvasEditValue::Json(Json::Text(text)) if text == "hd" || text == PREMIUM => {
            CanvasEditValue::Json(Json::Text(PREMIUM.to_owned()))
        }
        CanvasEditValue::Json(_) | CanvasEditValue::Bytes(_) | CanvasEditValue::Tuple => {
            value.clone()
        }
    };
    options.insert("quality".to_owned(), quality);
}

// Accepted for OpenAI compatibility and dropped; b64_json and None are the silent values.
fn take_response_format(options: &mut CanvasEditOptions) -> Option<IgnoredResponseFormat> {
    let value = take_value(options, "response_format")?;
    if let CanvasEditValue::Json(Json::Text(text)) = &value
        && text == B64_JSON
    {
        return None;
    }
    Some(IgnoredResponseFormat(value))
}

fn drop_unknown(options: &mut CanvasEditOptions) {
    options.retain(|name, _value| {
        name.starts_with('_')
            || SUPPORTED_OPENAI_PARAMS.contains(&name.as_str())
            || KEPT_PARAMS.contains(&name.as_str())
    });
}

// Python's %s rendering of the value; only a str reaches here through the declared OpenAI type.
fn ignored_text(value: &CanvasEditValue) -> String {
    match value {
        CanvasEditValue::Json(json) => text_of(json),
        CanvasEditValue::Bytes(content) => format!("b'{}'", content.escape_ascii()),
        CanvasEditValue::Tuple => "()".to_owned(),
    }
}

// The crate's Python int() port answers the two dimension values; a value it cannot read leaves the size unparsed.
fn int_json(text: &str) -> Option<Json> {
    parse_python_int(text)
        .ok()
        .map(Number::from)
        .map(Json::Number)
}
