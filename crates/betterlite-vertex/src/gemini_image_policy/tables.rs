use betterlite_bedrock::image_models::sizing::parse_python_int;

// name, ratio numerator and denominator, then the smallest listed size; LiteLLM lists that pair times 1, 2, 4 and 8
const RATIOS: [(&str, u64, u64, u64, u64); 14] = [
    ("1:1", 1, 1, 512, 512),
    ("1:4", 1, 4, 256, 1024),
    ("1:8", 1, 8, 192, 1536),
    ("2:3", 2, 3, 424, 632),
    ("3:2", 3, 2, 632, 424),
    ("3:4", 3, 4, 448, 600),
    ("4:1", 4, 1, 1024, 256),
    ("4:3", 4, 3, 600, 448),
    ("4:5", 4, 5, 464, 576),
    ("5:4", 5, 4, 576, 464),
    ("8:1", 8, 1, 1536, 192),
    ("9:16", 9, 16, 384, 688),
    ("16:9", 16, 9, 688, 384),
    ("21:9", 21, 9, 792, 336),
];
const SIZE_SCALES: [u64; 4] = [1, 2, 4, 8];

// these two sizes sit outside the scaled families above
const PORTRAIT_SIZES: [(u64, u64, &str); 2] = [(1280, 896, "4:3"), (896, 1280, "3:4")];

const AREA_512_LIMIT: u128 = 768 * 768;
const AREA_1K_LIMIT: u128 = 1536 * 1536;
const AREA_2K_LIMIT: u128 = 3072 * 3072;

// a dimension past this keeps every rational comparison inside u128
const MAX_DIMENSION: u64 = 0xFFFF_FFFF;

#[must_use]
pub fn parse_dimensions(size: &str) -> Option<(u64, u64)> {
    if size == "auto" {
        return None;
    }
    let lowered = size.to_lowercase();
    let (width_text, height_text) = lowered.split_once('x')?;
    let width = u64::try_from(parse_python_int(width_text).ok()?).ok()?;
    let height = u64::try_from(parse_python_int(height_text).ok()?).ok()?;
    let valid = 1..=MAX_DIMENSION;
    (valid.contains(&width) && valid.contains(&height)).then_some((width, height))
}

#[must_use]
pub fn aspect_ratio(width: u64, height: u64) -> &'static str {
    if let Some(ratio) = exact_ratio(width, height) {
        return ratio;
    }
    let requested = (u128::from(width), u128::from(height));
    let mut nearest: Option<(&'static str, (u128, u128))> = None;
    for (ratio, numerator, denominator, _, _) in RATIOS {
        let distance = (
            scaled(numerator, requested.1),
            scaled(denominator, requested.0),
        );
        // an exact tie keeps the larger ratio, as LiteLLM's rounded float logs do at 3:8
        let closer = nearest.is_none_or(|(_, best)| {
            distance_is_closer(distance, best)
                || (!distance_is_closer(best, distance) && fraction_less(best, distance))
        });
        if closer {
            nearest = Some((ratio, distance));
        }
    }
    nearest.map_or("1:1", |(ratio, _)| ratio)
}

fn exact_ratio(width: u64, height: u64) -> Option<&'static str> {
    if let Some((_, _, ratio)) = PORTRAIT_SIZES
        .iter()
        .find(|(known_width, known_height, _)| *known_width == width && *known_height == height)
    {
        return Some(*ratio);
    }
    RATIOS
        .iter()
        .find_map(|(ratio, _, _, base_width, base_height)| {
            SIZE_SCALES
                .iter()
                .any(|scale| {
                    scaled(*scale, u128::from(*base_width)) == u128::from(width)
                        && scaled(*scale, u128::from(*base_height)) == u128::from(height)
                })
                .then_some(*ratio)
        })
}

fn scaled(factor: u64, dimension: u128) -> u128 {
    u128::from(factor)
        .checked_mul(dimension)
        .unwrap_or(u128::MAX)
}

// compares |ln(left)| with |ln(right)| as exact rationals
fn distance_is_closer(left: (u128, u128), right: (u128, u128)) -> bool {
    let product = left.0.checked_mul(right.0).zip(left.1.checked_mul(right.1));
    match (left.0 >= left.1, right.0 >= right.1) {
        (true, true) => fraction_less(left, right),
        (false, false) => fraction_less(right, left),
        (true, false) => product.is_some_and(|(numerator, denominator)| numerator < denominator),
        (false, true) => product.is_some_and(|(numerator, denominator)| numerator > denominator),
    }
}

fn fraction_less(left: (u128, u128), right: (u128, u128)) -> bool {
    left.0
        .checked_mul(right.1)
        .zip(left.1.checked_mul(right.0))
        .is_some_and(|(left_cross, right_cross)| left_cross < right_cross)
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
