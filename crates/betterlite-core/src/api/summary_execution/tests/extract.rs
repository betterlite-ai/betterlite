use rstest::rstest;

use crate::api::summary_execution::extract::{
    CompletionAnswer, ExtractionError, SummaryUsage, extract_response_text, extract_usage,
};

fn parsed(raw: &str) -> CompletionAnswer {
    serde_json::from_str(raw).expect("a completion answer")
}

fn text(raw: &str) -> Option<String> {
    extract_response_text(&parsed(raw)).expect("the text reads")
}

fn usage(raw: &str) -> Result<SummaryUsage, ExtractionError> {
    extract_usage(&parsed(raw))
}

#[rstest]
#[case::no_fields("{}")]
#[case::null_choices(r#"{"choices":null}"#)]
#[case::empty_choices(r#"{"choices":[]}"#)]
#[case::no_message(r#"{"choices":[{}]}"#)]
#[case::null_message(r#"{"choices":[{"message":null}]}"#)]
#[case::no_content(r#"{"choices":[{"message":{}}]}"#)]
#[case::usage_only(r#"{"usage":{"prompt_tokens":4}}"#)]
fn response_text_needs_a_choice_and_a_message(#[case] raw: &str) {
    assert_eq!(text(raw), None, "{raw}");
}

#[test]
fn response_text_keeps_a_string_as_sent() {
    assert_eq!(
        text(r#"{"choices":[{"message":{"content":""}}]}"#),
        Some(String::new())
    );
    assert_eq!(
        text(r#"{"choices":[{"message":{"content":"hello"}}]}"#),
        Some(String::from("hello"))
    );
}

#[rstest]
#[case::number(r#"{"choices":[{"message":{"content":7}}]}"#)]
#[case::boolean(r#"{"choices":[{"message":{"content":true}}]}"#)]
#[case::object(r#"{"choices":[{"message":{"content":{"type":"text"}}}]}"#)]
#[case::null(r#"{"choices":[{"message":{"content":null}}]}"#)]
#[case::empty_list(r#"{"choices":[{"message":{"content":[]}}]}"#)]
fn response_text_ignores_nontext_content(#[case] raw: &str) {
    assert_eq!(text(raw), None, "{raw}");
}

#[test]
fn response_text_joins_only_text_parts() {
    let joined = r#"{"choices":[{"message":{"content":[
        {"type":"text","text":"A"},"skip",{"type":"image"},{"type":"text"},
        {"type":"text","text":"B"}]}}]}"#;
    assert_eq!(text(joined), Some(String::from("AB")));
    let nontext_part = r#"{"choices":[{"message":{"content":[
        {"type":"image","text":5},{"type":"text","text":"A"}]}}]}"#;
    assert_eq!(text(nontext_part), Some(String::from("A")));
    let empty = r#"{"choices":[{"message":{"content":[{"type":"image"},{"type":"text"}]}}]}"#;
    assert_eq!(text(empty), None);
}

#[test]
fn response_text_rejects_a_nonstring_text_part() {
    let raw = r#"{"choices":[{"message":{"content":[
        {"type":"text","text":"A"},{"type":"text","text":5}]}}]}"#;
    let error = extract_response_text(&parsed(raw)).expect_err("the part is not text");
    assert_eq!(error, ExtractionError::NonTextPart);
}

#[rstest]
#[case::no_usage("{}")]
#[case::null_usage(r#"{"usage":null}"#)]
#[case::empty_usage(r#"{"usage":{}}"#)]
#[case::null_counts(r#"{"usage":{"prompt_tokens":null,"completion_tokens":null}}"#)]
#[case::empty_strings(r#"{"usage":{"prompt_tokens":"","completion_tokens":""}}"#)]
#[case::zeros(r#"{"usage":{"prompt_tokens":0,"completion_tokens":0}}"#)]
fn usage_defaults_to_zero(#[case] raw: &str) {
    let counted = usage(raw).expect("zero counts");
    assert_eq!(
        (counted.input_tokens, counted.output_tokens),
        (0, 0),
        "{raw}"
    );
}

#[test]
fn usage_reads_numbers_and_numeric_strings() {
    let from_strings = usage(r#"{"usage":{"prompt_tokens":"12","completion_tokens":"3"}}"#)
        .expect("numeric strings");
    assert_eq!(
        (from_strings.input_tokens, from_strings.output_tokens),
        (12, 3)
    );
    let from_float =
        usage(r#"{"usage":{"prompt_tokens":12.9,"completion_tokens":3}}"#).expect("a float");
    assert_eq!((from_float.input_tokens, from_float.output_tokens), (12, 3));
    let from_bools = usage(r#"{"usage":{"prompt_tokens":true,"completion_tokens":false}}"#)
        .expect("bool counts");
    assert_eq!((from_bools.input_tokens, from_bools.output_tokens), (1, 0));
}

#[rstest]
#[case::word(r#"{"usage":{"prompt_tokens":"bad"}}"#)]
#[case::blank(r#"{"usage":{"prompt_tokens":" "}}"#)]
#[case::list(r#"{"usage":{"prompt_tokens":[1]}}"#)]
#[case::object(r#"{"usage":{"prompt_tokens":{"x":1}}}"#)]
#[case::negative(r#"{"usage":{"prompt_tokens":-1}}"#)]
fn usage_rejects_values_that_are_not_integers(#[case] raw: &str) {
    assert_eq!(
        usage(raw).expect_err("not an integer"),
        ExtractionError::TokenNotAnInteger,
        "{raw}"
    );
}
