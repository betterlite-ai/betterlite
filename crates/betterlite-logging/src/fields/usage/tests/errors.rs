use rstest::rstest;

use crate::fields::usage::UsageError;
use crate::fields::usage::parse::usage_from_mapping;

use super::decode_object;

#[rstest]
#[case::fractional_tokens(r#"{"prompt_tokens":2.5}"#, UsageError::NotAnInteger { field: "prompt_tokens" })]
#[case::text_tokens(r#"{"prompt_tokens":"2"}"#, UsageError::NotAnInteger { field: "prompt_tokens" })]
#[case::details_not_an_object(
    r#"{"prompt_tokens_details":[]}"#,
    UsageError::NotAnObject { field: "prompt_tokens_details" }
)]
#[case::completion_details_not_an_object(
    r#"{"completion_tokens_details":"x"}"#,
    UsageError::NotAnObject { field: "completion_tokens_details" }
)]
#[case::cost_not_a_number(r#"{"cost":"cheap"}"#, UsageError::NotANumber { field: "cost" })]
fn logging_fields_behavior_tests_behavior(#[case] mapping: &str, #[case] expected: UsageError) {
    assert_eq!(usage_from_mapping(&decode_object(mapping)), Err(expected));
}
