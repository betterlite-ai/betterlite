use betterlite_models::decisions::Json;

use super::VideoParams;
use super::profile::{VideoProfile, resolution_for_short_side};

pub const ALLOWED_ASPECT_RATIOS: [&str; 7] = ["auto", "16:9", "9:16", "1:1", "4:3", "3:4", "21:9"];

#[must_use]
pub fn size_params(size: &Json, profile: &VideoProfile) -> VideoParams {
    let Json::Text(raw_size) = size else {
        return VideoParams::new();
    };
    let normalized = raw_size.to_lowercase();
    if let Some(canonical) = canonical_resolution(&normalized, profile) {
        let mut params = VideoParams::new();
        params.insert("resolution".to_owned(), Json::Text(canonical.to_owned()));
        return params;
    }
    let Some((width, height)) = dimensions(&normalized) else {
        return VideoParams::new();
    };
    let Some((aspect_width, aspect_height)) = reduced_aspect(width, height) else {
        return VideoParams::new();
    };
    let aspect = format!("{aspect_width}:{aspect_height}");
    let resolution = resolution_for_short_side(width.min(height), profile);
    let mut params = VideoParams::new();
    params.insert("resolution".to_owned(), Json::Text(resolution.to_owned()));
    if ALLOWED_ASPECT_RATIOS.contains(&aspect.as_str()) {
        params.insert("aspect_ratio".to_owned(), Json::Text(aspect));
    }
    params
}

fn canonical_resolution<'profile>(
    normalized: &str,
    profile: &'profile VideoProfile,
) -> Option<&'profile str> {
    profile
        .resolutions
        .iter()
        .find(|resolution| resolution.to_lowercase() == normalized)
        .copied()
}

fn dimensions(normalized: &str) -> Option<(u128, u128)> {
    if normalized.matches('x').count() != 1 {
        return None;
    }
    let (width_text, height_text) = normalized.split_once('x')?;
    if !digits_only(width_text) || !digits_only(height_text) {
        return None;
    }
    let width: u128 = width_text.parse().ok()?;
    let height: u128 = height_text.parse().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

fn digits_only(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.bytes().all(|byte| byte.is_ascii_digit())
}

fn reduced_aspect(width: u128, height: u128) -> Option<(u128, u128)> {
    let divisor = greatest_common_divisor(width, height)?;
    let reduced_width = width.checked_div(divisor)?;
    let reduced_height = height.checked_div(divisor)?;
    Some((reduced_width, reduced_height))
}

fn greatest_common_divisor(mut left: u128, mut right: u128) -> Option<u128> {
    while right != 0 {
        let remainder = left.checked_rem(right)?;
        left = right;
        right = remainder;
    }
    Some(left)
}
