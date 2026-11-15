use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use num_traits::cast::cast;

use crate::api::fal::images::error::FalImageError;

const SIZE_ALIASES: [(&str, &str); 8] = [
    ("1024x1024", "1:1"),
    ("512x512", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("1024x768", "4:3"),
    ("768x1024", "3:4"),
    ("1280x960", "4:3"),
    ("960x1280", "3:4"),
];

pub fn translate_size(size: &Json) -> Result<(&'static str, Json), FalImageError> {
    let Json::Text(text) = size else {
        return Err(FalImageError::SizeNotText);
    };
    Ok((
        "aspect_ratio",
        Json::Text(map_aspect_ratio(text).to_owned()),
    ))
}

#[must_use]
pub fn map_aspect_ratio(size: &str) -> &'static str {
    if let Some((_, ratio)) = SIZE_ALIASES.iter().find(|(alias, _)| *alias == size) {
        return ratio;
    }
    parse_ratio(size).unwrap_or("1:1")
}

// Python splits on a bare x, so Fal's -x- separator reads as one malformed part there and falls to the default
fn parse_ratio(size: &str) -> Option<&'static str> {
    if size.contains("-x-") {
        return None;
    }
    let (width, height) = parse_image_dimensions(size).ok()?;
    if height == 0 {
        return None;
    }
    let width = cast::<i64, f64>(width)?;
    let height = cast::<i64, f64>(height)?;
    Some(ratio_for(width / height))
}

// the pinned order decides every overlap and both edges belong to the interval; a gap falls through to 1:1
fn ratio_for(ratio: f64) -> &'static str {
    if (0.95..=1.05).contains(&ratio) {
        return "1:1";
    }
    if ratio >= 1.7 {
        return "16:9";
    }
    if ratio <= 0.6 {
        return "9:16";
    }
    if (1.3..=1.4).contains(&ratio) {
        return "4:3";
    }
    if (0.7..=0.8).contains(&ratio) {
        return "3:4";
    }
    if (1.45..=1.55).contains(&ratio) {
        return "3:2";
    }
    if (0.65..=0.7).contains(&ratio) {
        return "2:3";
    }
    if (1.2..=1.3).contains(&ratio) {
        return "5:4";
    }
    if (0.75..=0.85).contains(&ratio) {
        return "4:5";
    }
    "1:1"
}
