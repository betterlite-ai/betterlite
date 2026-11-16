#![cfg(test)]

use rstest::rstest;

use super::apply;
use crate::model_armor::decisions::model_response::CallResponse;
use crate::model_armor::hooks::tests::support::{contents, model_response};

#[test]
fn response_masking_replaces_every_complete_choice_with_content() {
    let mut response = model_response(
        r#"{"choices":[{"message":{"content":"hello"}},{"message":{"content":"second"}}]}"#,
    );
    apply(
        &mut response,
        "hello\nsecond",
        Some("masked text".to_owned()),
    );
    assert_eq!(
        contents(&response),
        vec!["masked text".to_owned(), "masked text".to_owned()]
    );
}

#[test]
fn response_masking_leaves_an_empty_choice_and_a_streaming_delta() {
    let mut response = model_response(
        r#"{"choices":[{"message":{"content":""}},{"delta":{"content":"hello"}},{"message":{"content":"hello"}}]}"#,
    );
    apply(&mut response, "hellohello", Some("masked text".to_owned()));
    assert_eq!(
        contents(&response),
        vec![String::new(), "hello".to_owned(), "masked text".to_owned(),]
    );
}

#[test]
fn response_masking_ignores_a_non_chat_response() {
    let mut response = CallResponse::Other;
    apply(&mut response, "hello", Some("masked text".to_owned()));
    assert!(contents(&response).is_empty());
}

#[test]
fn response_masking_skips_an_identical_text() {
    let mut response = model_response(r#"{"choices":[{"message":{"content":"scanned"}}]}"#);
    apply(&mut response, "scanned", Some("scanned".to_owned()));
    assert_eq!(contents(&response), vec!["scanned".to_owned()]);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(String::new()))]
fn response_masking_skips_an_absent_or_empty_sanitized_text(#[case] sanitized: Option<String>) {
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    apply(&mut response, "hello", sanitized);
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
}
