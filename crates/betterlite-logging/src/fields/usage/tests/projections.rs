use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::{ResponseUsage, Usage, get_usage_as_dict};

use super::{decode, decode_object};

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
    assert_eq!(get_usage_as_dict(response, base.as_ref()), decode(expected));
}

#[test]
fn the_combined_usage_is_read_before_the_response_mapping() {
    let combined = usage(r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17}"#);
    let empty = IndexMap::new();
    let projected = get_usage_as_dict(ResponseUsage::Mapping(&empty), Some(&combined));
    let expected = serde_json::from_str::<Json>(
        r#"{"prompt_tokens":9,"completion_tokens":8,"total_tokens":17,"completion_tokens_details":null,"prompt_tokens_details":null}"#,
    )
    .unwrap();
    assert_eq!(projected, expected);
}
