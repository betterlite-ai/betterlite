use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_pre_post::args::RawBody;
use crate::logging_pre_post::classifier::classifier_input_snapshot;

fn body(text: &str) -> RawBody {
    serde_json::from_str(text).expect("the fixture is valid")
}

#[rstest]
#[case::object_as_sent(
    body(r#"{"model":"m","extra_body":{"a":1}}"#),
    false,
    Some(r#"{"model":"m","extra_body":{"a":1}}"#)
)]
#[case::sdk_merges_extra_body(
    body(r#"{"model":"m","extra_headers":{"h":1},"extra_query":{"q":1},"extra_body":{"a":1}}"#),
    true,
    Some(r#"{"model":"m","a":1}"#)
)]
#[case::sdk_without_extra_body(
    body(r#"{"model":"m","extra_query":{"q":1}}"#),
    true,
    Some(r#"{"model":"m"}"#)
)]
#[case::sdk_extra_body_wins(
    body(r#"{"model":"m","extra_body":{"model":"n"}}"#),
    true,
    Some(r#"{"model":"n"}"#)
)]
#[case::sdk_non_object_extra_body_is_dropped(
    body(r#"{"model":"m","extra_body":"x"}"#),
    true,
    Some(r#"{"model":"m"}"#)
)]
#[case::text_object(RawBody::Text(r#"{"model":"m"}"#.to_owned()), false, Some(r#"{"model":"m"}"#))]
#[case::text_object_ignores_sdk_merge(RawBody::Text(r#"{"extra_body":{"a":1}}"#.to_owned()), true, Some(r#"{"extra_body":{"a":1}}"#))]
#[case::text_list(RawBody::Text("[1]".to_owned()), false, None)]
#[case::text_invalid(RawBody::Text("nope".to_owned()), false, None)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] value: RawBody,
    #[case] openai_sdk: bool,
    #[case] expected_json: Option<&str>,
) {
    let expected: Option<IndexMap<String, Json>> =
        expected_json.map(|text| serde_json::from_str(text).expect("the fixture is valid"));
    assert_eq!(classifier_input_snapshot(&value, openai_sdk), expected);
}
