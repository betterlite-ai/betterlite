#![cfg(test)]

use super::content::responses_api_content;
use super::support::value;

#[test]
fn responses_content_returns_the_output_list() {
    let response = value(r#"{"output": [{"type": "message", "content": []}]}"#);
    assert_eq!(
        responses_api_content(&response),
        Some(value(r#"[{"type": "message", "content": []}]"#))
    );
}

#[test]
fn responses_content_is_none_for_an_empty_output() {
    assert_eq!(responses_api_content(&value(r#"{"output": []}"#)), None);
    assert_eq!(responses_api_content(&value(r#"{"id": "resp-1"}"#)), None);
    assert_eq!(responses_api_content(&value(r#"{"output": null}"#)), None);
}
