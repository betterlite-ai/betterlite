use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_pre_post::masking::masked_headers;

fn parsed(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture is valid")
}

#[rstest]
#[case::bearer_value(
    r#"{"Authorization":"Bearer sk-abcdef"}"#,
    r#"{"Authorization":"Be****ef"}"#
)]
#[case::four_chars_hidden_fully(r#"{"x-api-key":"abcd"}"#, r#"{"x-api-key":"*****"}"#)]
#[case::five_chars_keep_edges(r#"{"x-api-key":"abcde"}"#, r#"{"x-api-key":"ab****de"}"#)]
#[case::keyword_ignores_case(r#"{"X-API-KEY":"abcdefgh"}"#, r#"{"X-API-KEY":"ab****gh"}"#)]
#[case::plain_header_untouched(
    r#"{"content-type":"application/json"}"#,
    r#"{"content-type":"application/json"}"#
)]
#[case::number_untouched(r#"{"token":12345}"#, r#"{"token":12345}"#)]
#[case::list_untouched(r#"{"secret":["abcdefgh"]}"#, r#"{"secret":["abcdefgh"]}"#)]
#[case::nested_sensitive_key_masked(
    r#"{"credentials":{"password":"abcdefgh","note":"visible"}}"#,
    r#"{"credentials":{"password":"ab****gh","note":"visible"}}"#
)]
#[case::nested_plain_key_untouched(
    r#"{"secret":{"inner":"abcdefgh"}}"#,
    r#"{"secret":{"inner":"abcdefgh"}}"#
)]
#[case::character_edges(
    "{\"password\":\"\u{e9}abcdef\u{20ac}\"}",
    "{\"password\":\"\u{e9}a****f\u{20ac}\"}"
)]
fn logging_pre_post_behavior_tests_behavior(#[case] headers: &str, #[case] expected: &str) {
    assert_eq!(masked_headers(&parsed(headers), false), parsed(expected));
}

#[rstest]
#[case::masking_on(false, r#"{"token":"ab****gh"}"#)]
#[case::masking_skipped(true, r#"{"token":"abcdefgh"}"#)]
fn ignore_sensitive_headers_skips_masking(#[case] ignore: bool, #[case] expected: &str) {
    assert_eq!(
        masked_headers(&parsed(r#"{"token":"abcdefgh"}"#), ignore),
        parsed(expected)
    );
}
