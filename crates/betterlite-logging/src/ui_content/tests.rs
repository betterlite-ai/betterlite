#![cfg(test)]

use super::*;

#[test]
fn object_is_fields() {
    assert!(matches!(
        to_ui_content(r#"{"key":"value"}"#),
        UiContent::Fields { .. }
    ));
}

#[test]
fn message_is_message_content() {
    assert!(matches!(
        to_ui_content(r#"[{"role":"user","content":"hello"}]"#),
        UiContent::Messages { .. }
    ));
}

#[test]
fn fields_and_tool_calls_are_projected() {
    let content = to_ui_content(
        r#"[{"role":"assistant","content":null,"name":"bot","tool_calls":[{"function":{"name":"run","arguments":{"x":1}}}]}]"#,
    );
    let UiContent::Messages { messages } = content else {
        panic!("expected messages");
    };
    assert_eq!(messages[0].name.as_deref(), Some("bot"));
    assert_eq!(messages[0].tool_calls.as_ref().map(Vec::len), Some(1));
    assert!(matches!(
        to_ui_content(r#"{"a":[1]}"#),
        UiContent::Fields { .. }
    ));
}

#[test]
fn plain_is_text() {
    assert!(matches!(to_ui_content("plain"), UiContent::Text { .. }));
}
