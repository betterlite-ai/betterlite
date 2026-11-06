use rstest::rstest;

use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::project::usage_json;
use crate::fields::usage::{ResponseUsage, Usage, get_usage_from_response_obj};

use super::{decode, decode_object};

fn usage(mapping: &str) -> Usage {
    usage_from_mapping(&decode_object(mapping)).unwrap()
}

#[rstest]
#[case::absent(None, None, r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null}"#)]
#[case::mapping_is_parsed(
    Some(r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}"#),
    None,
    r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
#[case::combined_wins(
    Some("{}"),
    Some(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}"#),
    r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17,"completion_tokens_details":null,"prompt_tokens_details":null}"#
)]
fn logging_fields_behavior_tests_behavior(
    #[case] mapping: Option<&str>,
    #[case] combined: Option<&str>,
    #[case] expected: &str,
) {
    let base = combined.map(usage);
    let fields = mapping.map(decode_object);
    let response = fields
        .as_ref()
        .map_or(ResponseUsage::Absent, ResponseUsage::Mapping);
    let result = get_usage_from_response_obj(response, base.as_ref()).unwrap();
    assert_eq!(usage_json(&result), decode(expected));
}

#[test]
fn the_combined_usage_is_returned_before_an_invalid_mapping_is_parsed() {
    let combined = usage(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}"#);
    let mapping = decode_object(r#"{"prompt_tokens":2.5}"#);
    let result = get_usage_from_response_obj(ResponseUsage::Mapping(&mapping), Some(&combined));
    assert_eq!(result, Ok(combined));
}

#[test]
fn an_invalid_mapping_is_parsed_when_no_combined_usage_exists() {
    let mapping = decode_object(r#"{"prompt_tokens":2.5}"#);
    let _error = get_usage_from_response_obj(ResponseUsage::Mapping(&mapping), None).unwrap_err();
}

#[test]
fn a_typed_usage_is_returned_as_is() {
    let typed = usage(r#"{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}"#);
    assert_eq!(
        get_usage_from_response_obj(ResponseUsage::Usage(&typed), None),
        Ok(typed)
    );
}
