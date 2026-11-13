use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::{ResponseUsage, Usage, get_usage_as_dict};

use super::{convert, decode, decode_object};

fn usage(mapping: &str) -> Usage {
    usage_from_mapping(&decode_object(mapping)).unwrap()
}

#[rstest]
#[case::absent(
    None,
    None,
    r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#
)]
#[case::empty_mapping_is_logged_as_sent(Some("{}"), None, "{}")]
#[case::combined_wins(
    Some("{}"),
    Some(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}"#),
    r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
#[case::responses_usage_is_converted(
    Some(r#"{"input_tokens":2,"output_tokens":3}"#),
    None,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
#[case::interactions_usage_is_converted(
    Some(r#"{"total_input_tokens":2,"total_output_tokens":3}"#),
    None,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null,"cache_read_input_tokens":null}"#
)]
#[case::a_lone_input_tokens_key_is_not_provider_usage(
    Some(r#"{"input_tokens":2}"#),
    None,
    r#"{"input_tokens":2}"#
)]
#[case::a_lone_output_tokens_key_is_not_provider_usage(
    Some(r#"{"output_tokens":3}"#),
    None,
    r#"{"output_tokens":3}"#
)]
#[case::interactions_with_a_lone_output_tokens_key_stay_interactions(
    Some(r#"{"total_input_tokens":2,"output_tokens":3}"#),
    None,
    r#"{"prompt_tokens":2,"completion_tokens":0,"total_tokens":2,"completion_tokens_details":null,"prompt_tokens_details":null,"cache_read_input_tokens":null}"#
)]
#[case::interactions_reasoning_fills_the_completion_details(
    Some(r#"{"total_input_tokens":2,"total_output_tokens":3,"total_reasoning_tokens":1}"#),
    None,
    r#"{"prompt_tokens":2,"completion_tokens":4,"total_tokens":6,"completion_tokens_details":{"accepted_prediction_tokens":null,"audio_tokens":null,"reasoning_tokens":1,"rejected_prediction_tokens":null,"text_tokens":null,"image_tokens":null,"video_tokens":null},"prompt_tokens_details":null,"cache_read_input_tokens":null}"#
)]
fn logging_fields_behavior_tests_behavior(
    #[case] mapping: Option<&str>,
    #[case] combined: Option<&str>,
    #[case] expected: &str,
) {
    let base = combined.map(usage);
    let fields = mapping.map(decode_object);
    let response = fields.map_or(ResponseUsage::Absent, ResponseUsage::Mapping);
    let projected = get_usage_as_dict(response, base.as_ref(), &convert).unwrap();
    assert_eq!(projected, decode(expected));
}

#[test]
fn the_combined_usage_is_read_before_the_response_mapping() {
    let combined = usage(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}"#);
    let empty = IndexMap::new();
    let projected =
        get_usage_as_dict(ResponseUsage::Mapping(empty), Some(&combined), &convert).unwrap();
    let expected = serde_json::from_str::<Json>(
        r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17,"completion_tokens_details":null,"prompt_tokens_details":null}"#,
    )
    .unwrap();
    assert_eq!(projected, expected);
}
