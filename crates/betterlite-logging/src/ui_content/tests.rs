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
fn plain_is_text() {
    assert!(matches!(to_ui_content("plain"), UiContent::Text { .. }));
}
