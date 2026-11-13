use rstest::rstest;

use super::support;
use crate::fetch_policy::outcome::classify_fetch_response;

#[rstest]
#[case::truncated_utf16(&[0xFF, 0xFE, 0x7B], "truncated UTF-16")]
#[case::truncated_utf32(&[0xFF, 0xFE, 0x00, 0x00, 0x7B, 0x00], "truncated UTF-32")]
#[case::lone_surrogate(&[0xFF, 0xFE, 0x00, 0xD8], "invalid UTF-16 code unit")]
#[case::invalid_code_point(
    &[0xFF, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00],
    "invalid UTF-32 code point"
)]
fn catalog_fetch_policy_behavior_tests_behavior(#[case] body: &[u8], #[case] detail: &str) {
    let response = support::response_bytes(200, body.to_vec());
    let unavailable = support::unavailable(classify_fetch_response(response, support::URL));
    assert!(
        unavailable
            .reason
            .starts_with("invalid JSON from https://catalog.test/model.json")
    );
    assert!(unavailable.reason.contains(detail));
}
