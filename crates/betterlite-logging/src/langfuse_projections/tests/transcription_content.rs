#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt, value};

#[test]
fn transcription_content_returns_the_text() {
    let body = value(r#"{"text": "a transcript"}"#);
    let response = ResponseObj::Transcription(&body);
    let (input, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("atranscription"),
    );
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, Some(Json::Text("a transcript".to_owned())));
}

#[test]
fn transcription_content_is_none_without_text() {
    let body = value(r#"{"task": "transcribe"}"#);
    let response = ResponseObj::Transcription(&body);
    let (_, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("atranscription"),
    );
    assert_eq!(output, None);
}
