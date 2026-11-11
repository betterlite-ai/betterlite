#![cfg(test)]

use crate::chat::ChatRequest;

#[test]
fn a_null_disable_fallbacks_leaves_fallbacks_on() {
    let body = br#"{"model":"m","messages":[],"disable_fallbacks":null}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_ne!(request.disable_fallbacks, Some(true));
    let disabled = br#"{"model":"m","messages":[],"disable_fallbacks":true}"#;
    let disabled_request = ChatRequest::parse(disabled).unwrap();
    assert_eq!(disabled_request.disable_fallbacks, Some(true));
}

#[test]
fn the_category_fallback_overrides_are_read_and_never_forwarded() {
    let body = br#"{"model":"m","messages":[],"context_window_fallbacks":[{"gpt-4":["long-ctx"]}],"content_policy_fallbacks":[{"gpt-4":[{"model":"safe","messages":[]}]}]}"#;
    let request = ChatRequest::parse(body).unwrap();
    assert_eq!(
        serde_json::to_string(&request.context_window_fallbacks).unwrap(),
        r#"[{"gpt-4":["long-ctx"]}]"#
    );
    assert_eq!(
        serde_json::to_string(&request.content_policy_fallbacks).unwrap(),
        r#"[{"gpt-4":[{"model":"safe","messages":[]}]}]"#
    );
    let written = serde_json::to_string(&request).unwrap();
    assert!(!written.contains("context_window_fallbacks"));
    assert!(!written.contains("content_policy_fallbacks"));
    assert!(!written.contains("disable_fallbacks"));
}
