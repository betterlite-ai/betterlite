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

#[test]
fn response_text_needs_a_choice_and_a_message() {
    let missing = [
        r#"{}"#,
        r#"{"choices":null}"#,
        r#"{"choices":[]}"#,
        r#"{"choices":[{}]}"#,
        r#"{"choices":[{"message":null}]}"#,
        r#"{"choices":[{"message":{}}]}"#,
        r#"{"usage":{"prompt_tokens":4}}"#,
    ];
    for raw in missing {
        assert_eq!(text(raw), None, "{raw}");
    }
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

#[test]
fn response_text_ignores_nontext_content() {
    let nontext = [
        r#"{"choices":[{"message":{"content":7}}]}"#,
        r#"{"choices":[{"message":{"content":true}}]}"#,
        r#"{"choices":[{"message":{"content":{"type":"text"}}}]}"#,
        r#"{"choices":[{"message":{"content":null}}]}"#,
        r#"{"choices":[{"message":{"content":[]}}]}"#,
    ];
    for raw in nontext {
        assert_eq!(text(raw), None, "{raw}");
    }
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

#[test]
fn usage_defaults_to_zero() {
    let zero = [
        r#"{}"#,
        r#"{"usage":null}"#,
        r#"{"usage":{}}"#,
        r#"{"usage":{"prompt_tokens":null,"completion_tokens":null}}"#,
        r#"{"usage":{"prompt_tokens":"","completion_tokens":""}}"#,
        r#"{"usage":{"prompt_tokens":0,"completion_tokens":0}}"#,
    ];
    for raw in zero {
        let counted = usage(raw).expect("zero counts");
        assert_eq!(
            (counted.input_tokens, counted.output_tokens),
            (0, 0),
            "{raw}"
        );
    }
}

#[test]
fn usage_reads_numbers_and_numeric_strings() {
    let counted = usage(r#"{"usage":{"prompt_tokens":"12","completion_tokens":"3"}}"#)
        .expect("numeric strings");
    assert_eq!((counted.input_tokens, counted.output_tokens), (12, 3));
    let counted =
        usage(r#"{"usage":{"prompt_tokens":12.9,"completion_tokens":3}}"#).expect("a float");
    assert_eq!((counted.input_tokens, counted.output_tokens), (12, 3));
    let counted = usage(r#"{"usage":{"prompt_tokens":true,"completion_tokens":false}}"#)
        .expect("bool counts");
    assert_eq!((counted.input_tokens, counted.output_tokens), (1, 0));
}

#[test]
fn usage_rejects_values_that_are_not_integers() {
    let invalid = [
        r#"{"usage":{"prompt_tokens":"bad"}}"#,
        r#"{"usage":{"prompt_tokens":" "}}"#,
        r#"{"usage":{"prompt_tokens":[1]}}"#,
        r#"{"usage":{"prompt_tokens":{"x":1}}}"#,
        r#"{"usage":{"prompt_tokens":-1}}"#,
    ];
    for raw in invalid {
        assert_eq!(
            usage(raw).expect_err("not an integer"),
            ExtractionError::TokenNotAnInteger,
            "{raw}"
        );
    }
}
