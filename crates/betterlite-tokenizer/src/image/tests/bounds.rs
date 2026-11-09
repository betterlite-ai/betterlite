use rstest::rstest;

use crate::image::geometry::high_detail_upper_bound;
use crate::image::settings::Settings;

use super::support::{DEFAULT, SMALL_LIMITS};

#[rstest]
#[case::default_base(85, &DEFAULT, 1445)]
#[case::custom_base(100, &DEFAULT, 1700)]
#[case::custom_limits(10, &SMALL_LIMITS, 330)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] base_tokens: u64,
    #[case] settings: &Settings,
    #[case] expected: u64,
) {
    match high_detail_upper_bound(base_tokens, settings) {
        Ok(tokens) => assert_eq!(tokens, expected),
        Err(error) => panic!("expected {expected} tokens, got {error:?}"),
    }
}
