use rstest::rstest;

use super::support::json;
use crate::api::responses_parameters::stream_options::normalize_stream_options;

#[rstest]
#[case::boolean_true(r#"{"include_obfuscation": true}"#, Some(true))]
#[case::boolean_false(r#"{"include_obfuscation": false}"#, Some(false))]
#[case::empty_object("{}", None)]
#[case::null("null", None)]
#[case::include_usage(r#"{"include_usage": true}"#, None)]
#[case::integer(r#"{"include_obfuscation": 1}"#, None)]
#[case::text(r#"{"include_obfuscation": "true"}"#, None)]
#[case::extra_keys(r#"{"include_obfuscation": true, "other": 1}"#, Some(true))]
fn stream_options_keep_only_a_boolean_include_obfuscation(
    #[case] source: &str,
    #[case] expected: Option<bool>,
) {
    let value = json(source);
    let normalized = normalize_stream_options(Some(&value));
    match expected {
        Some(flag) => assert_eq!(
            normalized.and_then(|options| options.include_obfuscation.flatten()),
            Some(flag)
        ),
        None => assert!(normalized.is_none(), "{normalized:?}"),
    }
}

#[test]
fn an_absent_stream_options_value_is_null() {
    assert!(normalize_stream_options(None).is_none());
}
