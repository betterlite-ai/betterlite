use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;

use crate::api::fal::images::error::FalImageError;

// LiteLLM's ordered ratio bounds in percent (130 means 1.3); the first interval that holds the ratio wins
const RATIO_BOUNDS: [(Option<i64>, Option<i64>, &str); 9] = [
    (Some(95), Some(105), "1:1"),
    (Some(170), None, "16:9"),
    (None, Some(60), "9:16"),
    (Some(130), Some(140), "4:3"),
    (Some(70), Some(80), "3:4"),
    (Some(145), Some(155), "3:2"),
    (Some(65), Some(70), "2:3"),
    (Some(120), Some(130), "5:4"),
    (Some(75), Some(85), "4:5"),
];

const PERCENT: i128 = 100;

pub fn translate_size(size: &Json) -> Result<(&'static str, Json), FalImageError> {
    let Json::Text(text) = size else {
        return Err(FalImageError::SizeNotText);
    };
    Ok((
        "aspect_ratio",
        Json::Text(map_aspect_ratio(text).to_owned()),
    ))
}

// LiteLLM's eight exact aliases come first; every other size falls to the ratio cascade
#[must_use]
pub fn map_aspect_ratio(size: &str) -> &'static str {
    match size {
        "1024x1024" | "512x512" => "1:1",
        "1792x1024" => "16:9",
        "1024x1792" => "9:16",
        "1024x768" | "1280x960" => "4:3",
        "768x1024" | "960x1280" => "3:4",
        other => parse_ratio(other).unwrap_or("1:1"),
    }
}

// Python splits on a bare x, so Fal's -x- separator reads as one malformed part there and falls to the default
fn parse_ratio(size: &str) -> Option<&'static str> {
    if size.contains("-x-") {
        return None;
    }
    let (width, height) = parse_image_dimensions(size)
        .ok()
        .filter(|(_, dimension)| *dimension != 0)?;
    Some(ratio_for(width, height))
}

// a negative height would flip every cross product, so the ratio keeps both dimensions positive
fn ratio_for(width: i64, height: i64) -> &'static str {
    let sign = i128::from(height).signum();
    let ratio_width = i128::from(width).saturating_mul(sign);
    let ratio_height = i128::from(height).saturating_mul(sign);
    let scaled_width = ratio_width.saturating_mul(PERCENT);
    for (lower, upper, ratio) in RATIO_BOUNDS {
        let above = lower
            .is_none_or(|bound| scaled_width >= ratio_height.saturating_mul(i128::from(bound)));
        let below = upper
            .is_none_or(|bound| scaled_width <= ratio_height.saturating_mul(i128::from(bound)));
        if above && below {
            return ratio;
        }
    }
    "1:1"
}
