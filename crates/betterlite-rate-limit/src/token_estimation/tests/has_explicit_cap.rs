#![cfg(test)]

use rstest::rstest;

use super::output_cap::explicit_output_cap;
use super::support::request;

#[rstest]
#[case::explicit_zero(r#"{"max_tokens":0}"#, true)]
#[case::explicit_number(r#"{"max_tokens":10}"#, true)]
#[case::explicit_text(r#"{"max_tokens":"10"}"#, true)]
#[case::absent("{}", false)]
#[case::null(r#"{"max_tokens":null}"#, false)]
#[case::boolean(r#"{"max_tokens":false}"#, false)]
#[case::unparsable(r#"{"max_tokens":"wide"}"#, false)]
fn an_explicit_cap_is_any_parsed_value(#[case] body: &str, #[case] present: bool) {
    assert_eq!(
        explicit_output_cap(&request(body), Some("completion")).is_some(),
        present
    );
}

#[test]
fn an_explicit_zero_is_a_cap_and_not_an_absent_field() {
    let data = request(r#"{"messages":[],"max_tokens":0}"#);
    assert_eq!(explicit_output_cap(&data, Some("completion")), Some(0));
}
