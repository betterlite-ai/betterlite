use betterlite_models::image_cost::default_calculator::parse_image_dimensions;

const SIZE_ALIASES: [(&str, &str); 6] = [
    ("1024x1024", "1:1"),
    ("512x512", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("1024x768", "4:3"),
    ("768x1024", "3:4"),
];

const FALLBACK_ASPECT_RATIO: &str = "1:1";

// the pair is width and a positive height, so a negative height flips both signs instead of every comparison
pub fn positive_dimensions(size: &str) -> Option<(i128, i128)> {
    if !size.contains('x') || size.contains("-x-") {
        return None;
    }
    let (width, height) = parse_image_dimensions(size)
        .ok()
        .filter(|(_, height)| *height != 0)?;
    let sign = i128::from(height).signum();
    Some((
        i128::from(width).saturating_mul(sign),
        i128::from(height).saturating_mul(sign),
    ))
}

// the ordered thresholds leave ratios between 1.05 and 1.2 and between 0.8 and 0.95 unmatched, and those fall through to 1:1
#[must_use]
pub fn map_aspect_ratio(size: &str) -> &'static str {
    for (alias, ratio) in SIZE_ALIASES {
        if alias == size {
            return ratio;
        }
    }
    let Some((width, height)) = positive_dimensions(size) else {
        return FALLBACK_ASPECT_RATIO;
    };
    let scaled = width.saturating_mul(100);
    if scaled >= height.saturating_mul(95) && scaled <= height.saturating_mul(105) {
        return "1:1";
    }
    if scaled >= height.saturating_mul(170) {
        return "16:9";
    }
    if scaled <= height.saturating_mul(60) {
        return "9:16";
    }
    if scaled >= height.saturating_mul(120) {
        return "4:3";
    }
    if scaled <= height.saturating_mul(80) {
        return "3:4";
    }
    FALLBACK_ASPECT_RATIO
}
