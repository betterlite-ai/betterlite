mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use crate::api::fal::imagen::size::positive_dimensions;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::GENERIC_OPENAI_PARAMS;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = GENERIC_OPENAI_PARAMS;

const FALLBACK_ASPECT_RATIO: &str = "1:1";

const ENDPOINT_PREFIX: &str = "fal-ai/";

struct AspectRatio {
    name: &'static str,
    width: i128,
    height: i128,
}

// the order breaks ties: the first ratio at the smallest distance wins
const SUPPORTED_ASPECT_RATIOS: [AspectRatio; 10] = [
    AspectRatio {
        name: "21:9",
        width: 21,
        height: 9,
    },
    AspectRatio {
        name: "16:9",
        width: 16,
        height: 9,
    },
    AspectRatio {
        name: "3:2",
        width: 3,
        height: 2,
    },
    AspectRatio {
        name: "4:3",
        width: 4,
        height: 3,
    },
    AspectRatio {
        name: "5:4",
        width: 5,
        height: 4,
    },
    AspectRatio {
        name: "1:1",
        width: 1,
        height: 1,
    },
    AspectRatio {
        name: "4:5",
        width: 4,
        height: 5,
    },
    AspectRatio {
        name: "3:4",
        width: 3,
        height: 4,
    },
    AspectRatio {
        name: "2:3",
        width: 2,
        height: 3,
    },
    AspectRatio {
        name: "9:16",
        width: 9,
        height: 16,
    },
];

// the requested model is the endpoint; only a missing fal-ai/ prefix is added
#[must_use]
pub fn endpoint(model: &str) -> String {
    if model.starts_with(ENDPOINT_PREFIX) {
        model.to_owned()
    } else {
        format!("{ENDPOINT_PREFIX}{model}")
    }
}

// the translated destination key in the options suppresses the translation, so an existing aspect_ratio beats size
pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
) -> Result<ImageParams, FalImageError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        match name.as_str() {
            "response_format" => {}
            "n" => {
                if !mapped.contains_key("num_images") {
                    mapped.insert("num_images".to_owned(), value.clone());
                }
            }
            "size" => {
                if !mapped.contains_key("aspect_ratio") {
                    mapped.insert(
                        "aspect_ratio".to_owned(),
                        Json::Text(map_aspect_ratio(value).to_owned()),
                    );
                }
            }
            _ if drop_params || mapped.contains_key(name) => {}
            _ => {
                return Err(FalImageError::UnsupportedParameters {
                    parameters: vec![name.clone()],
                    model: model.to_owned(),
                });
            }
        }
    }
    Ok(mapped)
}

// a text size is parsed, anything else or any unparsable text is 1:1, and the nearest supported ratio wins
fn map_aspect_ratio(size: &Json) -> &'static str {
    let Json::Text(text) = size else {
        return FALLBACK_ASPECT_RATIO;
    };
    let Some((width, height)) = positive_dimensions(text) else {
        return FALLBACK_ASPECT_RATIO;
    };
    SUPPORTED_ASPECT_RATIOS
        .iter()
        .min_by(|left, right| {
            let left_gap = ratio_gap(left, width, height);
            let right_gap = ratio_gap(right, width, height);
            left_gap
                .saturating_mul(right.height)
                .cmp(&right_gap.saturating_mul(left.height))
        })
        .map_or(FALLBACK_ASPECT_RATIO, |ratio| ratio.name)
}

// |ratio - target| scaled by the ratio's height and the target's positive height, so gaps compare without fractions
const fn ratio_gap(ratio: &AspectRatio, width: i128, height: i128) -> i128 {
    ratio
        .width
        .saturating_mul(height)
        .saturating_sub(width.saturating_mul(ratio.height))
        .saturating_abs()
}
