#![cfg(test)]

use rstest::rstest;

use super::output_cap::explicit_output_cap;
use super::support::request;

#[rstest]
#[case::max_tokens(r#"{"max_tokens":100}"#, Some(100))]
#[case::max_completion_tokens(r#"{"max_completion_tokens":200}"#, Some(200))]
#[case::max_output_tokens(r#"{"max_output_tokens":300}"#, Some(300))]
#[case::highest_of_all(r#"{"max_tokens":100,"max_output_tokens":300}"#, Some(300))]
#[case::string_value(r#"{"max_tokens":"4096"}"#, Some(4096))]
#[case::zero_value(r#"{"max_tokens":0}"#, Some(0))]
#[case::null_value(r#"{"max_tokens":null}"#, None)]
#[case::boolean_value(r#"{"max_tokens":true}"#, None)]
#[case::unparsable_value(r#"{"max_tokens":"wide"}"#, None)]
fn without_a_call_type_three_fields_are_read(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(explicit_output_cap(&request(body), None), expected);
}

#[rstest]
#[case::max_tokens(r#"{"max_tokens":100}"#, Some(100))]
#[case::max_completion_tokens(r#"{"max_completion_tokens":200}"#, Some(200))]
#[case::max_output_tokens(r#"{"max_output_tokens":300}"#, None)]
fn a_named_call_type_reads_only_two_fields(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        explicit_output_cap(&request(body), Some("completion")),
        expected
    );
}

#[rstest]
#[case::below_the_minimum(r#"{"max_output_tokens":4}"#, Some(16))]
#[case::above_the_minimum(r#"{"max_output_tokens":100}"#, Some(100))]
#[case::at_the_minimum(r#"{"max_output_tokens":16}"#, Some(16))]
#[case::max_tokens_is_ignored(r#"{"max_tokens":100}"#, None)]
#[case::absent("{}", None)]
fn responses_caps_floors_at_sixteen(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        explicit_output_cap(&request(body), Some("responses")),
        expected
    );
}

#[test]
fn an_embedding_request_has_no_output_cap() {
    assert_eq!(
        explicit_output_cap(&request(r#"{"max_tokens":100}"#), Some("embedding")),
        None
    );
}

#[rstest]
#[case::config_field(r#"{"config":{"maxOutputTokens":50}}"#, Some(50))]
#[case::generation_config(r#"{"generationConfig":{"max_output_tokens":60}}"#, Some(60))]
#[case::highest_of_both(
    r#"{"generationConfig":{"maxOutputTokens":50,"max_output_tokens":70}}"#,
    Some(70)
)]
#[case::config_wins_over_generation_config(
    r#"{"config":{"maxOutputTokens":50},"generationConfig":{"maxOutputTokens":70}}"#,
    Some(50)
)]
#[case::non_mapping_config_hides_generation_config(
    r#"{"config":"none","generationConfig":{"maxOutputTokens":70}}"#,
    None
)]
#[case::boolean_cap(r#"{"config":{"maxOutputTokens":true}}"#, None)]
fn google_caps_come_from_the_generation_config(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        explicit_output_cap(&request(body), Some("generate_content")),
        expected
    );
}
