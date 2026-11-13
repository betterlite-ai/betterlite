use rstest::rstest;

use crate::image::geometry::resize_high_res;
use crate::image::settings::Settings;

use super::support::{DEFAULT, SMALL_LIMITS};

#[rstest]
#[case::both_at_short_limit(768, 768, &DEFAULT, (768, 768))]
#[case::landscape_at_limit(769, 768, &DEFAULT, (769, 768))]
#[case::portrait_at_limit(768, 769, &DEFAULT, (768, 769))]
#[case::inside_limits(2, 3, &DEFAULT, (2, 3))]
#[case::square_resized(4096, 4096, &DEFAULT, (768, 768))]
#[case::landscape_clamped(8000, 3072, &DEFAULT, (2000, 768))]
#[case::landscape_not_clamped(2001, 769, &DEFAULT, (1998, 768))]
#[case::portrait_not_clamped(769, 2001, &DEFAULT, (768, 1998))]
#[case::portrait_clamped_to_zero(1, 10000, &DEFAULT, (0, 2000))]
#[case::landscape_clamped_to_zero(10000, 1, &DEFAULT, (2000, 0))]
#[case::widest_header(0xFFFF_FFFF, 2, &DEFAULT, (2000, 0))]
#[case::tallest_header(2, 0xFFFF_FFFF, &DEFAULT, (0, 2000))]
#[case::zero_both(0, 0, &DEFAULT, (0, 0))]
#[case::scaled_long_at_limit(2605, 1000, &DEFAULT, (2000, 768))]
#[case::exact_integer_aspect(800, 6, &DEFAULT, (2000, 15))]
#[case::exact_integer_aspect_portrait(6, 800, &DEFAULT, (15, 2000))]
#[case::exact_integer_aspect_thirds(1200, 9, &DEFAULT, (2000, 15))]
#[case::exact_integer_aspect_sixtieths(800, 24, &DEFAULT, (2000, 60))]
#[case::custom_landscape(401, 100, &SMALL_LIMITS, (400, 99))]
#[case::custom_portrait(100, 401, &SMALL_LIMITS, (99, 400))]
#[case::custom_inside(50, 50, &SMALL_LIMITS, (50, 50))]
fn image_token_geometry_behavior_tests_behavior(
    #[case] width: u32,
    #[case] height: u32,
    #[case] settings: &Settings,
    #[case] expected: (u32, u32),
) {
    match resize_high_res(width, height, settings) {
        Ok(size) => assert_eq!(size, expected),
        Err(error) => panic!("expected {expected:?}, got {error:?}"),
    }
}
