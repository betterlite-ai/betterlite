use rstest::rstest;

use crate::image::geometry::tiles_needed;
use crate::image::settings::Settings;

use super::support::{DEFAULT, RECT_TILES, SMALL_LIMITS};

#[rstest]
#[case::exact_tile(512, 512, &DEFAULT, 1)]
#[case::one_pixel_over(513, 512, &DEFAULT, 2)]
#[case::one_pixel_down(512, 513, &DEFAULT, 2)]
#[case::four_tiles(1024, 1024, &DEFAULT, 4)]
#[case::largest_high_res(2000, 768, &DEFAULT, 8)]
#[case::portrait_high_res(768, 2000, &DEFAULT, 8)]
#[case::empty(0, 0, &DEFAULT, 0)]
#[case::max_width(0xFFFF_FFFF, 1, &DEFAULT, 0x0080_0000)]
#[case::custom_tiles(400, 100, &SMALL_LIMITS, 16)]
#[case::unequal_tiles(400, 100, &RECT_TILES, 8)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] resized_width: u32,
    #[case] resized_height: u32,
    #[case] settings: &Settings,
    #[case] expected: u64,
) {
    match tiles_needed(resized_width, resized_height, settings) {
        Ok(tiles) => assert_eq!(tiles, expected),
        Err(error) => panic!("expected {expected} tiles, got {error:?}"),
    }
}
