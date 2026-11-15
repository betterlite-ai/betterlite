use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_openai::openai_base_config::Headers;
use rstest::rstest;
use serde_json::json;

use super::super::decode::{
    BYTE_ORDER_MARK, SonioxReply, decode_soniox_reply, identified_resource,
};
use super::super::error::ReplyError;
use super::super::raise::provider_message;
use super::super::response::{transcript, transcription_meta};
use crate::api::api_boundaries::ApiLimits;

fn decode(body: &[u8]) -> SonioxReply {
    decode_soniox_reply(200, Headers::new(), body, &ApiLimits::default())
        .expect("the reply decodes")
}

fn corpus_name(path: &Path) -> String {
    path.file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn decodes_a_json_object() {
    let reply = decode(br#"{"id": "tx_1"}"#);

    assert_eq!(
        reply.object().expect("an object payload").get("id"),
        Some(&json!("tx_1"))
    );
    assert_eq!(reply.text, r#"{"id": "tx_1"}"#);
    assert_eq!(identified_resource(&reply).expect("the id").id, "tx_1");
}

#[test]
fn keeps_the_status_and_headers() {
    let headers = Headers::from([(String::from("X-Request-Id"), String::from("abc"))]);

    let reply =
        decode_soniox_reply(503, headers.clone(), b"{}", &ApiLimits::default()).expect("the reply");

    assert_eq!(reply.status, 503);
    assert_eq!(reply.headers, headers);
}

#[rstest]
#[case::scalar("42")]
#[case::json_string(r#""boom""#)]
#[case::json_null("null")]
#[case::list("[1, 2]")]
#[case::malformed("{")]
#[case::empty("")]
#[case::trailing(r#"{"id": "tx"} extra"#)]
fn keeps_the_raw_text_when_the_body_is_not_an_object(#[case] body: &str) {
    let reply = decode(body.as_bytes());

    assert!(reply.payload.is_none());
    assert_eq!(reply.text, body);
    assert!(matches!(reply.object(), Err(ReplyError::NotObject)));
    assert!(matches!(
        identified_resource(&reply),
        Err(ReplyError::NotObject)
    ));
}

#[test]
fn strips_the_byte_order_mark_for_json() {
    let mut body = BYTE_ORDER_MARK.to_vec();
    body.extend_from_slice(br#"{"id": "tx_bom"}"#);

    let reply = decode(&body);

    assert_eq!(identified_resource(&reply).expect("the id").id, "tx_bom");
    assert!(reply.text.starts_with('\u{feff}'));
}

#[test]
fn replaces_invalid_utf8_in_the_text() {
    let reply = decode(b"{\"id\":\"\xff\"}");

    assert!(reply.payload.is_none());
    assert!(reply.text.contains('\u{fffd}'));
    assert!(matches!(
        identified_resource(&reply),
        Err(ReplyError::NotObject)
    ));
}

#[test]
fn requires_an_id_string() {
    let missing = decode(br#"{"status": "completed"}"#);
    let wrong = decode(br#"{"id": 7}"#);
    let null = decode(br#"{"id": null}"#);

    assert!(matches!(
        identified_resource(&missing),
        Err(ReplyError::Missing { field: "id" })
    ));
    assert!(matches!(
        identified_resource(&wrong),
        Err(ReplyError::Shape { field: "id", .. })
    ));
    assert!(matches!(
        identified_resource(&null),
        Err(ReplyError::Shape { field: "id", .. })
    ));
}

#[test]
fn bounds_the_reply_bytes() {
    let limits = ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: 4,
        max_items: 4,
    };

    let at_limit = decode_soniox_reply(200, Headers::new(), b"null", &limits);
    let over = decode_soniox_reply(200, Headers::new(), b"null ", &limits);

    assert!(at_limit.is_ok());
    assert!(matches!(
        over,
        Err(ReplyError::TooLarge {
            observed: 5,
            limit: 4
        })
    ));
}

#[rstest]
fn replays_the_soniox_reply_corpus(#[files("fuzz/corpus/soniox_reply/*")] path: PathBuf) {
    let name = corpus_name(&path);
    let bytes = fs::read(&path).expect("a corpus file is readable");
    let reply = decode_soniox_reply(200, Headers::new(), &bytes, &ApiLimits::default())
        .expect("a bounded reply decodes");

    match name.as_str() {
        "resource-ok.json" => {
            assert_eq!(identified_resource(&reply).expect("the id").id, "tx_1");
        }
        "resource-missing-id.json" => {
            assert!(matches!(
                identified_resource(&reply),
                Err(ReplyError::Missing { .. })
            ));
        }
        "resource-malformed-id.json" => {
            assert!(matches!(
                identified_resource(&reply),
                Err(ReplyError::Shape { .. })
            ));
        }
        "transcript-ok.json" => {
            let decoded = transcript(&reply, &ApiLimits::default()).expect("the transcript");
            assert_eq!(decoded.text.as_deref(), Some("hello"));
            assert_eq!(decoded.tokens.len(), 1);
        }
        "transcript-null-tokens.json" => {
            let decoded = transcript(&reply, &ApiLimits::default()).expect("the transcript");
            assert!(decoded.tokens.is_empty());
            assert_eq!(decoded.text.as_deref(), Some("hello"));
        }
        "transcript-malformed-tokens.json" => {
            assert!(matches!(
                transcript(&reply, &ApiLimits::default()),
                Err(ReplyError::Field { .. })
            ));
        }
        "transcript-tokens-not-a-list.json" => {
            assert!(matches!(
                transcript(&reply, &ApiLimits::default()),
                Err(ReplyError::Shape { .. })
            ));
        }
        "status-fields.json" => {
            let meta = transcription_meta(&reply).expect("the meta");
            assert_eq!(meta.status.as_deref(), Some("completed"));
            assert_eq!(meta.audio_duration_ms, Some(json!(1500)));
        }
        "status-wrong-typed.json" => {
            assert!(matches!(
                transcription_meta(&reply),
                Err(ReplyError::Shape { .. })
            ));
        }
        "error-object.json" => {
            assert_eq!(provider_message(&reply), "quota");
        }
        "error-non-object.json" => {
            assert_eq!(provider_message(&reply), reply.text);
        }
        "scalar.json" => {
            assert!(reply.payload.is_none());
            assert_eq!(reply.text, "42");
        }
        "array.json" => {
            assert!(reply.payload.is_none());
        }
        "bom.json" => {
            assert_eq!(identified_resource(&reply).expect("the id").id, "tx_bom");
        }
        "invalid-utf8.bin" => {
            assert!(reply.payload.is_none());
            assert!(reply.text.contains('\u{fffd}'));
        }
        "empty.bin" => {
            assert!(reply.payload.is_none());
            assert_eq!(reply.text, "");
        }
        "trailing-garbage.json" => {
            assert!(reply.payload.is_none());
        }
        other => panic!("the seed {other} has no expected reply"),
    }
}
