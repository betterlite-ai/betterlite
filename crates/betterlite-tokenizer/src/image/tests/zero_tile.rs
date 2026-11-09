use rstest::rstest;

use crate::image::error::ImageError;
use crate::image::geometry::{high_detail_upper_bound, tiles_needed};

use super::support::ZERO_TILE;

#[rstest]
#[case::tiles(400, 100)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] resized_width: u32,
    #[case] resized_height: u32,
) {
    let tiles = tiles_needed(resized_width, resized_height, &ZERO_TILE);
    assert!(
        matches!(&tiles, Err(ImageError::ZeroTile)),
        "expected a zero tile error, got {tiles:?}"
    );
    let bound = high_detail_upper_bound(85, &ZERO_TILE);
    assert!(
        matches!(&bound, Err(ImageError::ZeroTile)),
        "expected a zero tile error, got {bound:?}"
    );
}
