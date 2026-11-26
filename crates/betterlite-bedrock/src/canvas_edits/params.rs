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

// LiteLLM copies the options, rewrites size, n, quality and response_format, then drops unknown keys.
#[must_use]
pub fn map_openai_params(options: &CanvasEditOptions, drop_params: bool) -> MappedOptions {
    let mut mapped = options.clone();
    if let Some(size) = mapped.size.take()
        && let Some((width, height)) = size_pair(&size)
    {
        mapped.width = Some(CanvasEditValue::Json(width));
        mapped.height = Some(CanvasEditValue::Json(height));
    }
    if let Some(n) = mapped.n.take()
        && !matches!(n, CanvasEditValue::Json(Json::Null))
    {
        mapped.number_of_images = Some(n);
    }
    if let Some(quality) = mapped.quality.take()
        && !matches!(quality, CanvasEditValue::Json(Json::Null))
    {
        mapped.quality = Some(mapped_quality(quality));
    }
    let ignored_response_format = take_response_format(&mut mapped);
    if drop_params {
        mapped
            .passthrough
            .retain(|name, _value| name.starts_with('_'));
    }
    MappedOptions {
        options: mapped,
        ignored_response_format,
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

// hd and premium both mean premium; every other value is kept as given for the API to judge.
fn mapped_quality(value: CanvasEditValue) -> CanvasEditValue {
    match value {
        CanvasEditValue::Json(Json::Text(text)) if text == "hd" || text == PREMIUM => {
            CanvasEditValue::Json(Json::Text(PREMIUM.to_owned()))
        }
        CanvasEditValue::Json(json) => CanvasEditValue::Json(json),
        CanvasEditValue::Bytes(content) => CanvasEditValue::Bytes(content),
        CanvasEditValue::Tuple => CanvasEditValue::Tuple,
    }
}

// Accepted for OpenAI compatibility and dropped; b64_json and None are the silent values.
fn take_response_format(options: &mut CanvasEditOptions) -> Option<IgnoredResponseFormat> {
    let value = options.response_format.take()?;
    if matches!(value, CanvasEditValue::Json(Json::Null)) {
        return None;
    }
    if let CanvasEditValue::Json(Json::Text(text)) = &value
        && text == B64_JSON
    {
        return None;
    }
    Some(IgnoredResponseFormat(value))
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
