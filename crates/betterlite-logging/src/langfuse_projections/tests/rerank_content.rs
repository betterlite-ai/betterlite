#![cfg(test)]

use betterlite_models::decisions::Json;

use super::content::ResponseObj;
use super::input_output::get_input_output_content;
use super::support::{content_request, prompt, value};

#[test]
fn rerank_content_returns_the_results() {
    let body = value(r#"{"results": [{"index": 0, "relevance_score": 0.9}]}"#);
    let response = ResponseObj::Rerank(&body);
    let (input, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("arerank"));
    assert_eq!(input, Some(Json::Object(prompt())));
    assert_eq!(
        output,
        Some(value(r#"[{"index": 0, "relevance_score": 0.9}]"#))
    );
}

#[test]
fn rerank_content_is_none_without_results() {
    let body = value(r#"{"id": "rerank-1"}"#);
    let response = ResponseObj::Rerank(&body);
    let (_, output) =
        get_input_output_content(&prompt(), Some(&response), &content_request("arerank"));
    assert_eq!(output, None);
}
