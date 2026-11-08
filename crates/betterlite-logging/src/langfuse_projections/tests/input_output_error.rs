#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::{ContentRequest, get_input_output_content};
use super::support::{prompt, value};

#[test]
fn input_output_error_returns_the_prompt_and_status_message() {
    let body = value(r#"{"choices": [{"message": {"content": "ignored"}}]}"#);
    let response = ResponseObj::Chat(&body);
    let request = ContentRequest {
        call_type: Some("acompletion"),
        input: None,
        level: "ERROR",
        status_message: Some("provider rejected the request"),
    };
    let (input, output) = get_input_output_content(&prompt(), Some(&response), &request);
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(
        output,
        Some(Json::Text("provider rejected the request".to_owned()))
    );
}

#[test]
fn input_output_error_returns_the_status_message_without_a_response() {
    let request = ContentRequest {
        call_type: Some("acompletion"),
        input: None,
        level: "ERROR",
        status_message: Some("provider rejected the request"),
    };
    let (input, output) = get_input_output_content(&prompt(), None, &request);
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(
        output,
        Some(Json::Text("provider rejected the request".to_owned()))
    );
}

#[test]
fn input_output_error_without_a_message_falls_through_to_the_response() {
    let body = value(r#"{"choices": [{"message": {"role": "assistant"}}]}"#);
    let response = ResponseObj::Chat(&body);
    let request = ContentRequest {
        call_type: Some("acompletion"),
        input: None,
        level: "ERROR",
        status_message: None,
    };
    let (_, output) = get_input_output_content(&prompt(), Some(&response), &request);
    assert_eq!(output, Some(value(r#"{"role": "assistant"}"#)));
}

#[test]
fn input_output_error_level_is_case_sensitive() {
    let response = ResponseObj::Speech;
    let request = ContentRequest {
        call_type: Some("aspeech"),
        input: None,
        level: "error",
        status_message: Some("lower case is not LiteLLM's ERROR"),
    };
    let (_, output) = get_input_output_content(&prompt(), Some(&response), &request);
    assert_eq!(output, Some(Json::Text("speech-output".to_owned())));
}
