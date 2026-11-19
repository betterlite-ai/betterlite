use rstest::rstest;
use serde_json::Value;
use serde_json::json;

use crate::passthrough::websocket::json_object_frame;

#[test]
fn an_object_frame_is_decoded() {
    assert_eq!(
        json_object_frame(br#"{"setup":{"model":"m"},"stream":true}"#).map(Value::Object),
        Some(json!({"setup": {"model": "m"}, "stream": true}))
    );
}

#[test]
fn an_empty_object_frame_is_still_a_frame() {
    assert_eq!(json_object_frame(b"{}").map(Value::Object), Some(json!({})));
}

#[rstest]
#[case::array(b"[1,2]")]
#[case::string(b"\"text\"")]
#[case::number(b"3")]
#[case::null(b"null")]
#[case::unterminated(b"{")]
#[case::empty(b"")]
#[case::invalid_utf8(b"{\"a\":\"\xff\"}")]
fn frames_that_are_not_objects_are_rejected(#[case] frame: &[u8]) {
    assert_eq!(json_object_frame(frame), None);
}
