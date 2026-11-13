#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt};

#[test]
fn embedding_content_has_the_prompt_as_input_and_no_output() {
    let response = ResponseObj::Embedding;
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("embedding"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(output, None);
}
