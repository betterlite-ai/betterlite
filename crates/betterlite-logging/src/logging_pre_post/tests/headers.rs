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
    false,
    r#"{"Authorization":"Be****ef"}"#
)]
#[case::four_chars_hidden_fully(r#"{"x-api-key":"abcd"}"#, false, r#"{"x-api-key":"*****"}"#)]
#[case::five_chars_keep_edges(r#"{"x-api-key":"abcde"}"#, false, r#"{"x-api-key":"ab****de"}"#)]
#[case::keyword_ignores_case(r#"{"X-API-KEY":"abcdefgh"}"#, false, r#"{"X-API-KEY":"ab****gh"}"#)]
#[case::plain_header_untouched(
    r#"{"content-type":"application/json"}"#,
    false,
    r#"{"content-type":"application/json"}"#
)]
#[case::number_untouched(r#"{"token":12345}"#, false, r#"{"token":12345}"#)]
#[case::list_untouched(r#"{"secret":["abcdefgh"]}"#, false, r#"{"secret":["abcdefgh"]}"#)]
#[case::nested_sensitive_key_masked(
    r#"{"credentials":{"password":"abcdefgh","note":"visible"}}"#,
    false,
    r#"{"credentials":{"password":"ab****gh","note":"visible"}}"#
)]
#[case::nested_plain_key_untouched(
    r#"{"secret":{"inner":"abcdefgh"}}"#,
    false,
    r#"{"secret":{"inner":"abcdefgh"}}"#
)]
#[case::character_edges(
    "{\"password\":\"\u{e9}abcdef\u{20ac}\"}",
    false,
    "{\"password\":\"\u{e9}a****f\u{20ac}\"}"
)]
#[case::masking_on(r#"{"token":"abcdefgh"}"#, false, r#"{"token":"ab****gh"}"#)]
#[case::masking_skipped(r#"{"token":"abcdefgh"}"#, true, r#"{"token":"abcdefgh"}"#)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] headers: &str,
    #[case] ignore: bool,
    #[case] expected: &str,
) {
    assert_eq!(masked_headers(&parsed(headers), ignore), parsed(expected));
}
