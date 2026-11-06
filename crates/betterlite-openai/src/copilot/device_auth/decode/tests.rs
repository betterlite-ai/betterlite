#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::json;

use super::{
    AccessReply, DeviceReply, REPLY_LIMIT, ReplyError, decode_access_reply, decode_device_reply,
};

#[test]
fn device_reply_decodes_typed_and_additional_fields() {
    let reply: DeviceReply = decode_device_reply(
        br#"{"device_code":"d","user_code":"u","verification_uri":"v","interval":5,"expires_in":900}"#,
    )
    .unwrap();

    assert_eq!(reply.device_code.as_deref(), Some("d"));
    assert_eq!(reply.user_code.as_deref(), Some("u"));
    assert_eq!(reply.verification_uri.as_deref(), Some("v"));
    assert_eq!(reply.additional_fields.get("interval"), Some(&json!(5_i32)));
    assert_eq!(
        reply.additional_fields.get("expires_in"),
        Some(&json!(900_i32))
    );
    assert_eq!(reply.additional_fields.len(), 2);
}

#[rstest]
#[case::array(b"[]".as_slice())]
#[case::truncated(b"{".as_slice())]
#[case::wrong_type(br#"{"device_code": 5}"#.as_slice())]
fn device_reply_rejects_malformed_json(#[case] bytes: &[u8]) {
    assert!(matches!(
        decode_device_reply(bytes),
        Err(ReplyError::Json(_))
    ));
}

#[test]
fn device_reply_is_bounded_by_its_limit() {
    let bounded = format!("{{}}{}", " ".repeat(REPLY_LIMIT - 2));
    decode_device_reply(bounded.as_bytes()).unwrap();
    assert!(matches!(
        decode_device_reply(format!("{bounded} ").as_bytes()),
        Err(ReplyError::Size { limit: REPLY_LIMIT })
    ));
}

#[test]
fn access_reply_reads_presence_and_accepts_an_empty_token() {
    let empty: AccessReply = decode_access_reply(br#"{"access_token":""}"#).unwrap();
    assert_eq!(empty.access_token.unwrap().expose_secret(), "");

    let missing: AccessReply = decode_access_reply(b"{}").unwrap();
    assert!(missing.access_token.is_none());
    assert!(missing.error.is_none());

    let pending: AccessReply =
        decode_access_reply(br#"{"error":"authorization_pending"}"#).unwrap();
    assert_eq!(pending.error.as_deref(), Some("authorization_pending"));
}

#[test]
fn access_reply_is_bounded_by_its_limit() {
    let bounded = format!("{{}}{}", " ".repeat(REPLY_LIMIT - 2));
    decode_access_reply(bounded.as_bytes()).unwrap();
    assert!(matches!(
        decode_access_reply(format!("{bounded} ").as_bytes()),
        Err(ReplyError::Size { limit: REPLY_LIMIT })
    ));
}

#[rstest]
fn replays_the_device_reply_corpus(#[files("fuzz/corpus/device_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let _device = decode_device_reply(&bytes);
    let _access = decode_access_reply(&bytes);
}
