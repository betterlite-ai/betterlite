#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::{ResponseObj, SPEECH_OUTPUT};
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt};

#[test]
fn speech_content_is_the_speech_output_marker() {
    let response = ResponseObj::Speech;
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("aspeech"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, Some(Json::Text(SPEECH_OUTPUT.to_owned())));
}
