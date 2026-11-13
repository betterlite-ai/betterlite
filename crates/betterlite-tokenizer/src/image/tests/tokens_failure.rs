use rstest::rstest;

use crate::image::error::ImageError;
use crate::image::tokens::{Mode, calculate_img_tokens};

use super::support::{
    BAD_BASE64_URL, DEFAULT, NO_PREFIX_URL, PNG_TRUNCATED_URL, PNG_ZERO_WIDTH_URL,
};

#[derive(Debug, Clone, Copy)]
enum Failure {
    Media,
    Truncated,
    ZeroSide,
}

#[rstest]
#[case::missing_prefix(NO_PREFIX_URL, Failure::Media)]
#[case::invalid_base64(BAD_BASE64_URL, Failure::Media)]
#[case::truncated_png(PNG_TRUNCATED_URL, Failure::Truncated)]
#[case::zero_width(PNG_ZERO_WIDTH_URL, Failure::ZeroSide)]
fn image_token_geometry_behavior_tests_behavior(#[case] data: &str, #[case] expected: Failure) {
    let result = calculate_img_tokens(data, Mode::High, 85, false, &DEFAULT);
    let Err(error) = result else {
        panic!("expected a failure, got a token count");
    };
    let matched = matches!(
        (&error, expected),
        (ImageError::Media(_), Failure::Media)
            | (ImageError::Truncated { .. }, Failure::Truncated)
            | (ImageError::ZeroSide, Failure::ZeroSide)
    );
    assert!(matched, "expected {expected:?}, got {error:?}");
}
