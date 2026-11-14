#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value};

use super::{TogetherCompletionError, project_prompt, transform_completion_request};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn message(content: &str) -> Value {
    value(&format!(r#"{{"role": "user", "content": {content}}}"#))
}

#[test]
fn a_single_text_message_becomes_its_string() {
    let request =
        transform_completion_request("together/llama", &[message(r#""hello""#)], Map::new())
            .unwrap();
    assert_eq!(request.get("prompt"), Some(&value(r#""hello""#)));
}

#[rstest]
#[case::two_messages(vec![message(r#""a""#), message(r#""b""#)])]
#[case::three_messages(vec![message(r#""a""#), message(r#""b""#), message(r#""c""#)])]
fn several_messages_are_refused(#[case] messages: Vec<Value>) {
    let error = transform_completion_request("together/llama", &messages, Map::new()).unwrap_err();
    assert!(matches!(error, TogetherCompletionError::MultiplePrompts));
    assert_eq!(
        error.to_string(),
        "TogetherAI does not support multiple prompts."
    );
}

#[rstest]
#[case::tokens("[1, 2, 3]")]
#[case::list_of_tokens("[[1, 2], [3]]")]
#[case::one_token("[1]")]
fn token_content_is_refused(#[case] content: &str) {
    let error = transform_completion_request("together/llama", &[message(content)], Map::new())
        .unwrap_err();
    assert!(matches!(error, TogetherCompletionError::Integers));
    assert_eq!(
        error.to_string(),
        "TogetherAI does not support integers as input"
    );
}

#[test]
fn an_empty_message_list_hits_the_token_rule() {
    let error = transform_completion_request("together/llama", &[], Map::new()).unwrap_err();
    assert!(matches!(error, TogetherCompletionError::Integers));
}

#[test]
fn a_one_string_list_becomes_that_string() {
    assert_eq!(
        project_prompt(value(r#"["only"]"#)).unwrap(),
        value(r#""only""#)
    );
}

#[test]
fn a_two_string_list_is_refused() {
    let error = project_prompt(value(r#"["a", "b"]"#)).unwrap_err();
    assert!(matches!(error, TogetherCompletionError::MultiplePrompts));
}

#[test]
fn an_empty_list_is_refused_as_integers() {
    let error = project_prompt(value("[]")).unwrap_err();
    assert!(matches!(error, TogetherCompletionError::Integers));
}

#[test]
fn a_plain_string_prompt_is_kept() {
    assert_eq!(
        project_prompt(value(r#""kept""#)).unwrap(),
        value(r#""kept""#)
    );
}

#[test]
fn the_request_overlays_optional_params_over_model_and_prompt() {
    let mut optional_params = Map::new();
    optional_params.insert(String::from("temperature"), value("0.5"));
    optional_params.insert(String::from("model"), value(r#""together/other""#));
    optional_params.insert(String::from("prompt"), value(r#""other prompt""#));
    let request =
        transform_completion_request("together/llama", &[message(r#""hello""#)], optional_params)
            .unwrap();
    assert_eq!(request.get("model"), Some(&value(r#""together/other""#)));
    assert_eq!(request.get("prompt"), Some(&value(r#""other prompt""#)));
    assert_eq!(request.get("temperature"), Some(&value("0.5")));
    assert_eq!(request.len(), 3);
}

#[test]
fn the_request_keeps_the_prompt_rule() {
    let error = transform_completion_request("together/llama", &[message("[1, 2]")], Map::new())
        .unwrap_err();
    assert!(matches!(error, TogetherCompletionError::Integers));
}
