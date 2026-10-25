#![cfg(test)]
use crate::decisions::decisions_text;
use crate::decisions::types::DecisionsJson;

#[test]
fn decisions_text_handles_json_and_strings() {
    assert_eq!(
        decisions_text(&DecisionsJson::String("text".to_owned())),
        "text"
    );
    let object: DecisionsJson = serde_json::from_str(r#"{"a":1}"#).expect("object");
    assert_eq!(decisions_text(&object), r#"{"a":1}"#);
}
