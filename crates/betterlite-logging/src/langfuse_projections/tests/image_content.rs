#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt, value};

#[test]
fn image_content_returns_the_data_list() {
    let body = value(r#"{"data": [{"url": "https://example.test/cat.png"}]}"#);
    let response = ResponseObj::Image(&body);
    let (input, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("image_generation"),
    );
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(
        output,
        Some(value(r#"[{"url": "https://example.test/cat.png"}]"#))
    );
}

#[test]
fn image_content_is_none_without_data() {
    let body = value(r#"{"created": 1}"#);
    let response = ResponseObj::Image(&body);
    let (_, output) = get_input_output_content(
        &prompt(),
        Some(&response),
        &content_request("image_generation"),
    );
    assert_eq!(output, None);
}
