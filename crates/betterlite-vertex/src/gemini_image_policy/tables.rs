use betterlite_bedrock::image_models::sizing::parse_python_int;

// LiteLLM's dict order decides a tie between two equally distant ratios
const ASPECT_RATIOS: [(&str, u64, u64); 14] = [
    ("1:1", 1, 1),
    ("1:4", 1, 4),
    ("1:8", 1, 8),
    ("2:3", 2, 3),
    ("3:2", 3, 2),
    ("3:4", 3, 4),
    ("4:1", 4, 1),
    ("4:3", 4, 3),
    ("4:5", 4, 5),
    ("5:4", 5, 4),
    ("8:1", 8, 1),
    ("9:16", 9, 16),
    ("16:9", 16, 9),
    ("21:9", 21, 9),
];

const SIZE_TO_ASPECT_RATIO: [(u64, u64, &str); 58] = [
    (512, 512, "1:1"),
    (1024, 1024, "1:1"),
    (2048, 2048, "1:1"),
    (4096, 4096, "1:1"),
    (256, 1024, "1:4"),
    (512, 2048, "1:4"),
    (1024, 4096, "1:4"),
    (2048, 8192, "1:4"),
    (192, 1536, "1:8"),
    (384, 3072, "1:8"),
    (768, 6144, "1:8"),
    (1536, 12288, "1:8"),
    (424, 632, "2:3"),
    (848, 1264, "2:3"),
    (1696, 2528, "2:3"),
    (3392, 5056, "2:3"),
    (632, 424, "3:2"),
    (1264, 848, "3:2"),
    (2528, 1696, "3:2"),
    (5056, 3392, "3:2"),
    (448, 600, "3:4"),
    (896, 1200, "3:4"),
    (1792, 2400, "3:4"),
    (3584, 4800, "3:4"),
    (1024, 256, "4:1"),
    (2048, 512, "4:1"),
    (4096, 1024, "4:1"),
    (8192, 2048, "4:1"),
    (600, 448, "4:3"),
    (1200, 896, "4:3"),
    (2400, 1792, "4:3"),
    (4800, 3584, "4:3"),
    (464, 576, "4:5"),
    (928, 1152, "4:5"),
    (1856, 2304, "4:5"),
    (3712, 4608, "4:5"),
    (576, 464, "5:4"),
    (1152, 928, "5:4"),
    (2304, 1856, "5:4"),
    (4608, 3712, "5:4"),
    (1536, 192, "8:1"),
    (3072, 384, "8:1"),
    (6144, 768, "8:1"),
    (12288, 1536, "8:1"),
    (384, 688, "9:16"),
    (768, 1376, "9:16"),
    (1536, 2752, "9:16"),
    (3072, 5504, "9:16"),
    (688, 384, "16:9"),
    (1376, 768, "16:9"),
    (2752, 1536, "16:9"),
    (5504, 3072, "16:9"),
    (792, 336, "21:9"),
    (1584, 672, "21:9"),
    (3168, 1344, "21:9"),
    (6336, 2688, "21:9"),
    (1280, 896, "4:3"),
    (896, 1280, "3:4"),
];

const AREA_512_LIMIT: u128 = 589_824;
const AREA_1K_LIMIT: u128 = 2_359_296;
const AREA_2K_LIMIT: u128 = 9_437_184;

// a dimension past this keeps every rational comparison inside u128
const MAX_DIMENSION: u64 = 4_294_967_295;

#[must_use]
pub fn parse_dimensions(size: &str) -> Option<(u64, u64)> {
    if size == "auto" {
        return None;
    }
    let lowered = size.to_lowercase();
    let (width_text, height_text) = lowered.split_once('x')?;
    Some((dimension(width_text)?, dimension(height_text)?))
}

fn dimension(text: &str) -> Option<u64> {
    let value = u64::try_from(parse_python_int(text).ok()?).ok()?;
    (value > 0 && value <= MAX_DIMENSION).then_some(value)
}

#[must_use]
pub fn aspect_ratio(width: u64, height: u64) -> &'static str {
    if let Some((_, _, ratio)) = SIZE_TO_ASPECT_RATIO
        .iter()
        .find(|(known_width, known_height, _)| *known_width == width && *known_height == height)
    {
        return ratio;
    }
    let requested = (u128::from(width), u128::from(height));
    let mut nearest: Option<(&'static str, (u128, u128))> = None;
    for (ratio, numerator, denominator) in ASPECT_RATIOS {
        let distance = (
            scaled(numerator, requested.1),
            scaled(denominator, requested.0),
        );
        if nearest.is_none_or(|(_, best)| distance_is_closer(distance, best)) {
            nearest = Some((ratio, distance));
        }
    }
    nearest.map_or("1:1", |(ratio, _)| ratio)
}

fn scaled(factor: u64, dimension: u128) -> u128 {
    u128::from(factor)
        .checked_mul(dimension)
        .unwrap_or(u128::MAX)
}

// compares |ln(left)| with |ln(right)| as exact rationals
fn distance_is_closer(left: (u128, u128), right: (u128, u128)) -> bool {
    match (left.0 >= left.1, right.0 >= right.1) {
        (true, true) => fraction_less(left, right),
        (false, false) => fraction_less(right, left),
        (true, false) => fraction_product(left, right)
            .is_some_and(|(numerator, denominator)| numerator < denominator),
        (false, true) => fraction_product(left, right)
            .is_some_and(|(numerator, denominator)| numerator > denominator),
    }
}

fn fraction_less(left: (u128, u128), right: (u128, u128)) -> bool {
    left.0
        .checked_mul(right.1)
        .zip(left.1.checked_mul(right.0))
        .is_some_and(|(left_cross, right_cross)| left_cross < right_cross)
}

fn fraction_product(left: (u128, u128), right: (u128, u128)) -> Option<(u128, u128)> {
    left.0.checked_mul(right.0).zip(left.1.checked_mul(right.1))
}

#[must_use]
pub fn gemini_image_size(width: u64, height: u64) -> &'static str {
    let area = scaled(width, u128::from(height));
    if area < AREA_512_LIMIT {
        "512"
    } else if area < AREA_1K_LIMIT {
        "1K"
    } else if area < AREA_2K_LIMIT {
        "2K"
    } else {
        "4K"
    }
}
