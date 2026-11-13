use rstest::rstest;

use crate::logging_pre_post::masking::mask_api_base;

#[rstest]
#[case::no_marker("https://api.example.com/v1", "https://api.example.com/v1")]
#[case::empty("", "")]
#[case::key_query("https://h/v1?key=abcdef1234", "https://h/v1?key=*****1234")]
#[case::first_marker_wins("https://h/?key=a&key=bcdefg", "https://h/?key=*****defg")]
#[case::tail_counts_characters("key=\u{e9}\u{20ac}", "key=*****y=\u{e9}\u{20ac}")]
#[case::marker_is_case_sensitive("https://h/?KEY=abcd", "https://h/?KEY=abcd")]
fn logging_pre_post_behavior_tests_behavior(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(mask_api_base(api_base), expected);
}
