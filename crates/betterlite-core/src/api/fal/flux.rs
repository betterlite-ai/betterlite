use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::api::fal::images::decode::{FalImage, decode_fal_image_reply};
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::GENERIC_OPENAI_PARAMS;

mod tests;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = GENERIC_OPENAI_PARAMS;

const HIDDEN_METADATA: [&str; 3] = ["seed", "timings", "has_nsfw_concepts"];

const ULTRA_ASPECT_RATIOS: [(&str, &str); 10] = [
    ("1024x1024", "1:1"),
    ("512x512", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("1024x768", "4:3"),
    ("768x1024", "3:4"),
    ("1536x1024", "3:2"),
    ("1024x1536", "2:3"),
    ("2048x876", "21:9"),
    ("876x2048", "9:21"),
];

const IMAGE_SIZES: [(&str, &str); 6] = [
    ("1024x1024", "square_hd"),
    ("512x512", "square"),
    ("1792x1024", "landscape_16_9"),
    ("1024x1792", "portrait_16_9"),
    ("1024x768", "landscape_4_3"),
    ("768x1024", "portrait_4_3"),
];

const FALLBACK_ASPECT_RATIO: &str = "16:9";

const FALLBACK_IMAGE_SIZE: &str = "landscape_4_3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluxProfile {
    Ultra,
    ProV11,
    Schnell,
    Dev,
}

#[derive(Debug, Error)]
pub enum FluxError {
    #[error("parameter size must be text to map to a Flux aspect ratio")]
    SizeNotText,
    #[error(
        "parameter {parameter} is not supported for model {model}; supported parameters are n, response_format and size; set drop_params to drop unsupported parameters"
    )]
    UnsupportedParameter { parameter: String, model: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FluxImageResponse {
    pub data: Vec<FalImage>,
    pub hidden_params: HiddenParams,
}

impl FluxProfile {
    #[must_use]
    pub const fn endpoint(self) -> &'static str {
        match self {
            Self::Ultra => "fal-ai/flux-pro/v1.1-ultra",
            Self::ProV11 => "fal-ai/flux-pro/v1.1",
            Self::Schnell => "fal-ai/flux/schnell",
            Self::Dev => "fal-ai/flux/dev",
        }
    }

    #[must_use]
    pub const fn size_parameter(self) -> &'static str {
        match self {
            Self::Ultra => "aspect_ratio",
            Self::ProV11 | Self::Schnell | Self::Dev => "image_size",
        }
    }

    pub fn map_openai_params(
        self,
        non_default: &ImageParams,
        optional: ImageParams,
        model: &str,
        drop_params: bool,
    ) -> Result<ImageParams, FluxError> {
        let mut mapped = optional;
        for (name, value) in non_default {
            if mapped.contains_key(name) {
                continue;
            }
            match name.as_str() {
                "n" => {
                    mapped.insert("num_images".to_owned(), value.clone());
                }
                "response_format" => {
                    mapped.insert("output_format".to_owned(), output_format(value));
                }
                "size" => {
                    mapped.insert(
                        self.size_parameter().to_owned(),
                        self.translate_size(value)?,
                    );
                }
                _ if drop_params => {}
                _ => {
                    return Err(FluxError::UnsupportedParameter {
                        parameter: name.clone(),
                        model: model.to_owned(),
                    });
                }
            }
        }
        Ok(mapped)
    }

    fn translate_size(self, value: &Json) -> Result<Json, FluxError> {
        match (self, value) {
            (Self::Ultra, Json::Text(text)) => Ok(Json::Text(map_aspect_ratio(text).to_owned())),
            (Self::Ultra, _) => Err(FluxError::SizeNotText),
            (Self::ProV11 | Self::Schnell | Self::Dev, size) => Ok(map_image_size(size)),
        }
    }
}

#[must_use]
fn output_format(value: &Json) -> Json {
    match value {
        Json::Text(text) if text == "url" || text == "b64_json" => Json::Text("jpeg".to_owned()),
        other => other.clone(),
    }
}

#[must_use]
pub fn map_aspect_ratio(size: &str) -> &'static str {
    for (alias, ratio) in ULTRA_ASPECT_RATIOS {
        if alias == size {
            return ratio;
        }
    }
    if !size.contains('x') || size.contains("-x-") {
        return FALLBACK_ASPECT_RATIO;
    }
    let Ok((width, height)) = parse_image_dimensions(size) else {
        return FALLBACK_ASPECT_RATIO;
    };
    if height == 0 {
        return FALLBACK_ASPECT_RATIO;
    }
    let scaled_width = i128::from(width);
    let scaled_height = i128::from(height);
    let (horizontal, vertical) = if scaled_height < 0 {
        (
            scaled_width.saturating_neg(),
            scaled_height.saturating_neg(),
        )
    } else {
        (scaled_width, scaled_height)
    };
    ratio_interval(horizontal, vertical).unwrap_or(FALLBACK_ASPECT_RATIO)
}

// LiteLLM's ordered float branches compared exactly; i64 dimensions times 23 fit in i128, so nothing saturates
const fn ratio_interval(horizontal: i128, vertical: i128) -> Option<&'static str> {
    if horizontal.saturating_mul(20) >= vertical.saturating_mul(19)
        && horizontal.saturating_mul(20) <= vertical.saturating_mul(21)
    {
        return Some("1:1");
    }
    if horizontal.saturating_mul(10) >= vertical.saturating_mul(23) {
        return Some("21:9");
    }
    if horizontal.saturating_mul(10) >= vertical.saturating_mul(17) {
        return Some("16:9");
    }
    if horizontal.saturating_mul(10) >= vertical.saturating_mul(13) {
        return Some("4:3");
    }
    if horizontal.saturating_mul(5) >= vertical.saturating_mul(3)
        && horizontal.saturating_mul(10) < vertical.saturating_mul(7)
    {
        return Some("3:4");
    }
    if horizontal.saturating_mul(20) >= vertical.saturating_mul(13)
        && horizontal.saturating_mul(4) < vertical.saturating_mul(3)
    {
        return Some("2:3");
    }
    if horizontal.saturating_mul(2) >= vertical
        && horizontal.saturating_mul(10) < vertical.saturating_mul(6)
    {
        return Some("9:16");
    }
    if horizontal.saturating_mul(2) < vertical {
        return Some("9:21");
    }
    None
}

#[must_use]
pub fn map_image_size(size: &Json) -> Json {
    let Json::Text(text) = size else {
        return size.clone();
    };
    for (alias, image_size) in IMAGE_SIZES {
        if alias == text.as_str() {
            return Json::Text(image_size.to_owned());
        }
    }
    if !text.contains('x') || text.contains("-x-") {
        return Json::Text(FALLBACK_IMAGE_SIZE.to_owned());
    }
    let Ok((width, height)) = parse_image_dimensions(text) else {
        return Json::Text(FALLBACK_IMAGE_SIZE.to_owned());
    };
    Json::Object(IndexMap::from([
        ("width".to_owned(), Json::Number(Number::from(width))),
        ("height".to_owned(), Json::Number(Number::from(height))),
    ]))
}

pub fn project_response(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
    response: &mut FluxImageResponse,
) -> Result<(), FalImageError> {
    let reply: Json =
        serde_json::from_slice(body).map_err(|source| FalImageError::ReplyDecode {
            status,
            headers: headers.clone(),
            source,
        })?;
    let Json::Object(fields) = reply else {
        return Err(FalImageError::ReplyNotAnObject {
            status,
            headers: headers.clone(),
        });
    };
    response
        .data
        .extend(decode_fal_image_reply(status, headers, body)?);
    for name in HIDDEN_METADATA {
        if let Some(value) = fields.get(name) {
            response
                .hidden_params
                .insert(name.to_owned(), value.clone());
        }
    }
    Ok(())
}
