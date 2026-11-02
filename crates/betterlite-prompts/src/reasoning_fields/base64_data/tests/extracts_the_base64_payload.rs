use super::*;
use rstest::rstest;

#[rstest]
#[case::data_url_with_base64("data:image/png;base64,iVBOR==", "iVBOR==")]
#[case::plain_http_url_unchanged("https://example.com/cat.png", "https://example.com/cat.png")]
#[case::data_url_without_base64_marker_unchanged(
    "data:image/png,iVBOR==",
    "data:image/png,iVBOR=="
)]
#[case::marker_before_data_prefix_unchanged(";base64,iVBOR==", ";base64,iVBOR==")]
#[case::empty_string("", "")]
#[case::only_the_marker_after_the_prefix("data:;base64,", "")]
fn reasoning_fields_behavior_tests_behavior(#[case] image_url: &str, #[case] expected: &str) {
    assert_eq!(extract_base64_data(image_url), expected);
}
