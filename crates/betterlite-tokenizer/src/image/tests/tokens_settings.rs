use rstest::rstest;

use crate::image::settings::Settings;
use crate::image::tokens::{Mode, calculate_img_tokens};

use super::support::{CUSTOM_DEFAULTS, DEFAULT_COUNT_123, PNG_1X1_URL, UNKNOWN_URL};

#[rstest]
#[case::default_count_custom(PNG_1X1_URL, true, &DEFAULT_COUNT_123, 123)]
#[case::unknown_custom_dims(UNKNOWN_URL, false, &CUSTOM_DEFAULTS, 425)]
fn image_token_geometry_behavior_tests_behavior(
    #[case] data: &str,
    #[case] use_default_image_token_count: bool,
    #[case] settings: &Settings,
    #[case] expected: u64,
) {
    match calculate_img_tokens(
        data,
        Mode::High,
        85,
        use_default_image_token_count,
        settings,
    ) {
        Ok(tokens) => assert_eq!(tokens, expected),
        Err(error) => panic!("expected {expected} tokens, got {error:?}"),
    }
}
