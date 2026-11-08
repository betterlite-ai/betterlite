#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::{ContentRequest, get_input_output_content};
use super::support::{content_request, prompt, value};

#[test]
fn input_output_dispatch_returns_the_prompt_and_chat_content() {
    let body = value(r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}]}"#);
    let response = ResponseObj::Chat(&body);
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("acompletion"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(
        output,
        Some(value(r#"{"role": "assistant", "content": "hi"}"#))
    );
}

#[test]
fn input_output_dispatch_returns_the_text_completion_text() {
    let body = value(r#"{"choices": [{"text": "hi"}]}"#);
    let response = ResponseObj::TextCompletion(&body);
    let (_, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("text_completion"),
    );
    assert_eq!(output, Some(Json::Text("hi".to_owned())));
}

#[test]
fn input_output_dispatch_returns_nothing_without_a_response() {
    let (input, output) =
        get_input_output_content(&prompt(), None, &content_request("acompletion"));
    assert_eq!(input, None);
    assert_eq!(output, None);
}

#[test]
fn dispatch_arealtime_list_uses_the_kwargs_input() {
    let items = value(r#"[{"type": "input_audio_buffer.append"}]"#);
    let kwargs_input = value(r#"{"audio": "AAAA"}"#);
    let response = ResponseObj::List(&items);
    let request = ContentRequest {
        call_type: Some("_arealtime"),
        input: Some(&kwargs_input),
        level: "DEFAULT",
        status_message: None,
    };
    let (input, output) = get_input_output_content(&prompt(), Some(&response), &request);
    assert_eq!(input, Some(kwargs_input.clone()));
    assert_eq!(output, Some(items.clone()));
}

#[test]
fn dispatch_arealtime_requires_the_call_type() {
    let items = value(r#"[{"type": "input_audio_buffer.append"}]"#);
    let response = ResponseObj::List(&items);
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("acompletion"));
    assert_eq!(input, None);
    assert_eq!(output, None);
}

#[test]
fn dispatch_pass_through_reads_the_response_field() {
    let body = value(r#"{"response": {"ok": true}}"#);
    let response = ResponseObj::Dict(&body);
    let (input, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("pass_through_endpoint"),
    );
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, Some(value(r#"{"ok": true}"#)));
}

#[test]
fn dispatch_pass_through_without_a_response_field_is_an_empty_string() {
    let body = value(r#"{"status": 200}"#);
    let response = ResponseObj::Dict(&body);
    let (_, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("pass_through_endpoint"),
    );
    assert_eq!(output, Some(Json::Text(String::new())));
}

#[test]
fn dispatch_pass_through_requires_the_call_type() {
    let body = value(r#"{"response": {"ok": true}}"#);
    let response = ResponseObj::Dict(&body);
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("acompletion"));
    assert_eq!(input, None);
    assert_eq!(output, None);
}
