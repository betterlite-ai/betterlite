#![cfg(test)]

use rstest::rstest;

use super::output_cap::output_candidate_count;
use super::support::request;

#[rstest]
#[case::absent("{}", 1)]
#[case::one(r#"{"n":1}"#, 1)]
#[case::n(r#"{"n":3}"#, 3)]
#[case::best_of(r#"{"best_of":4}"#, 4)]
#[case::highest_of_n_and_best_of(r#"{"n":2,"best_of":5}"#, 5)]
#[case::zero_n(r#"{"n":0}"#, 1)]
#[case::negative_n(r#"{"n":-5}"#, 1)]
#[case::text_n(r#"{"n":"4"}"#, 4)]
#[case::unparsable_text_n(r#"{"n":"many"}"#, 1)]
#[case::float_n(r#"{"n":2.9}"#, 2)]
#[case::boolean_n(r#"{"n":true}"#, 1)]
#[case::list_n(r#"{"n":[3]}"#, 1)]
fn chat_candidate_counts_max_over_the_aliases(#[case] body: &str, #[case] expected: i64) {
    assert_eq!(
        output_candidate_count(&request(body), Some("completion")),
        expected
    );
}

#[rstest]
#[case::candidate_count(
    r#"{"config":{"candidateCount":2},"n":1}"#,
    Some("generate_content"),
    2
)]
#[case::candidate_count_snake(r#"{"config":{"candidate_count":3}}"#, Some("generate_content"), 3)]
#[case::highest(
    r#"{"config":{"candidateCount":2,"candidate_count":4}}"#,
    Some("generate_content"),
    4
)]
#[case::generation_config(
    r#"{"generationConfig":{"candidateCount":2}}"#,
    Some("generate_content"),
    2
)]
#[case::ignored_for_chat(r#"{"config":{"candidateCount":2}}"#, Some("completion"), 1)]
fn google_candidate_counts_read_the_generation_config(
    #[case] body: &str,
    #[case] call_type: Option<&str>,
    #[case] expected: i64,
) {
    assert_eq!(output_candidate_count(&request(body), call_type), expected);
}

#[test]
fn without_a_call_type_the_generation_config_is_not_read() {
    assert_eq!(
        output_candidate_count(&request(r#"{"config":{"candidateCount":2}}"#), None),
        1
    );
}
