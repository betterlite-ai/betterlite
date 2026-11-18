#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::extract_usage_for_ocr_call;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

const EMPTY: &str =
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":0}"#;

#[rstest]
#[case::neither_source(None, None, "{}")]
#[case::null_usage_info(Some("null"), None, "{}")]
#[case::absent_usage_info_with_null_attribute(Some("null"), Some("null"), "{}")]
#[case::mapping_pages(
    Some(r#"{"pages_processed":2}"#),
    None,
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":2}"#
)]
#[case::mapping_overrides_counters(
    Some(r#"{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10,"pages_processed":1}"#),
    None,
    r#"{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10,"pages_processed":1}"#
)]
#[case::mapping_without_pages_defaults_to_zero(
    Some(r#"{"prompt_tokens":4}"#),
    None,
    r#"{"prompt_tokens":4,"completion_tokens":0,"total_tokens":0,"pages_processed":0}"#
)]
#[case::empty_mapping_still_returns_counters(Some("{}"), None, EMPTY)]
#[case::empty_mapping_falls_back_to_the_attribute(
    Some("{}"),
    Some(r#"{"pages_processed":5}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":5}"#
)]
#[case::null_usage_info_falls_back_to_the_attribute(
    Some("null"),
    Some(r#"{"pages_processed":4}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":4}"#
)]
#[case::truthy_mapping_wins_over_the_attribute(
    Some(r#"{"pages_processed":1}"#),
    Some(r#"{"pages_processed":9}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":1}"#
)]
#[case::text_usage_info(Some(r#""pages""#), None, EMPTY)]
#[case::falsy_zero_falls_back_to_the_attribute(
    Some("0"),
    Some(r#"{"pages_processed":3}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":3}"#
)]
#[case::truthy_number_is_not_an_object(Some("1"), None, EMPTY)]
#[case::empty_list_is_not_an_object(Some("[]"), None, EMPTY)]
#[case::falsy_attribute_is_not_an_object(None, Some("0"), EMPTY)]
#[case::truthy_text(Some(r#""pages""#), Some(r#"{"pages_processed":3}"#), EMPTY)]
#[case::empty_text(
    Some(r#""""#),
    Some(r#"{"pages_processed":3}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":3}"#
)]
#[case::truthy_list(Some(r#"[1]"#), Some(r#"{"pages_processed":3}"#), EMPTY)]
#[case::empty_list_fallback(
    Some(r#"[]"#),
    Some(r#"{"pages_processed":3}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":3}"#
)]
#[case::true_usage(Some(r#"true"#), Some(r#"{"pages_processed":3}"#), EMPTY)]
#[case::false_usage(
    Some(r#"false"#),
    Some(r#"{"pages_processed":3}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":3}"#
)]
#[case::truthy_number_fallback(Some(r#"2"#), Some(r#"{"pages_processed":3}"#), EMPTY)]
#[case::tiny_number(Some(r#"1e-999"#), Some(r#"{"pages_processed":3}"#), EMPTY)]
#[case::zero_with_exponent(
    Some(r#"0e99"#),
    Some(r#"{"pages_processed":3}"#),
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"pages_processed":3}"#
)]
fn extracts_pages_instead_of_tokens(
    #[case] usage_info: Option<&str>,
    #[case] attribute_usage_info: Option<&str>,
    #[case] expected: &str,
) {
    let usage = usage_info.map(json);
    let attribute_usage = attribute_usage_info.map(json);
    assert_eq!(
        extract_usage_for_ocr_call(usage.as_ref(), attribute_usage.as_ref()),
        object(expected)
    );
}
