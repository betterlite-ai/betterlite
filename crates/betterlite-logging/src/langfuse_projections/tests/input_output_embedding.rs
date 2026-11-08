#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt, value};

#[test]
fn input_output_embedding_call_type_has_no_output() {
    let body = value(r#"{"choices": [{"message": {"content": "not an embedding"}}]}"#);
    let response = ResponseObj::Chat(&body);
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("embedding"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, None);
}

#[test]
fn input_output_embedding_response_has_no_output() {
    let response = ResponseObj::Embedding;
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("aembedding"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, None);
}

#[test]
fn input_output_embedding_without_a_response_returns_nothing() {
    let (input, output) = get_input_output_content(&prompt(), None, &content_request("embedding"));
    assert_eq!(input, None);
    assert_eq!(output, None);
}
