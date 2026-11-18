#![cfg(test)]

use super::reply_options;

#[test]
fn reply_json_mode_is_retained_and_wrong_types_are_named() {
    let parsed = reply_options(&serde_json::from_str(r#"{"json_mode":true}"#).unwrap()).unwrap();
    assert_eq!(parsed.json_mode, Some(true));
    assert!(parsed.tools.is_none());
    let bad = reply_options(&serde_json::from_str(r#"{"json_mode":"true"}"#).unwrap());
    assert!(matches!(
        bad,
        Err(crate::converse_call::error::ConverseCallError::Param { field: "json_mode" })
    ));
}
