use rstest::rstest;

use crate::image::error::ImageError;
use crate::image::geometry::resize_high_res;
use crate::image::settings::Settings;

use super::support::{DEFAULT, SMALL_LIMITS};

#[rstest]
#[case::zero_width(0, 1000, &DEFAULT)]
#[case::zero_height(1000, 0, &DEFAULT)]
#[case::custom_zero_width(0, 401, &SMALL_LIMITS)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] width: u32,
    #[case] height: u32,
    #[case] settings: &Settings,
) {
    let result = resize_high_res(width, height, settings);
    assert!(
        matches!(&result, Err(ImageError::ZeroSide)),
        "expected a zero side error, got {result:?}"
    );
}
