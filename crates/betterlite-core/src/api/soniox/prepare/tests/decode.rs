use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Headers;
use rstest::rstest;
use serde_json::json;

use super::super::decode::{
    BYTE_ORDER_MARK, SonioxReply, decode_soniox_reply, identified_resource,
};
use super::super::error::ReplyError;
use super::super::raise::provider_message;
use super::super::response::{TranscriptionMeta, transcript, transcription_meta};
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

    let at_limit = decode_soniox_reply(200, Headers::new(), b"null", &limits)
        .expect("the reply at the byte limit decodes");
    let over = decode_soniox_reply(200, Headers::new(), b"null ", &limits);

    assert_eq!(at_limit.status, 200);
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

    if name.starts_with("resource-") || name == "bom.json" {
        expect_resource(&name, &reply);
    } else if name.starts_with("transcript-") {
        expect_transcript(&name, &reply);
    } else if name.starts_with("status-") {
        expect_status(&name, &reply);
    } else {
        expect_text(&name, &reply);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Missing,
    Shape,
    Field,
    Other,
}

const RESOURCE_REPLIES: &[(&str, Result<&str, Failure>)] = &[
    ("resource-ok.json", Ok("tx_1")),
    ("bom.json", Ok("tx_bom")),
    ("resource-missing-id.json", Err(Failure::Missing)),
    ("resource-malformed-id.json", Err(Failure::Shape)),
];

const TRANSCRIPT_REPLIES: &[(&str, Result<(Option<&str>, usize), Failure>)] = &[
    ("transcript-ok.json", Ok((Some("hello"), 1))),
    ("transcript-null-tokens.json", Ok((Some("hello"), 0))),
    ("transcript-malformed-tokens.json", Err(Failure::Field)),
    ("transcript-tokens-not-a-list.json", Err(Failure::Shape)),
];

const STATUS_REPLIES: &[(&str, Result<(Option<&str>, Option<i64>), Failure>)] = &[
    ("status-fields.json", Ok((Some("completed"), Some(1500)))),
    ("status-wrong-typed.json", Err(Failure::Shape)),
];

#[derive(Clone, Copy)]
enum TextReply {
    Message(&'static str),
    MessageIsText,
    NotObject,
    NotObjectText(&'static str),
    ReplacedText,
    EmptyText,
}

const TEXT_REPLIES: &[(&str, TextReply)] = &[
    ("error-object.json", TextReply::Message("quota")),
    ("error-non-object.json", TextReply::MessageIsText),
    ("scalar.json", TextReply::NotObjectText("42")),
    ("array.json", TextReply::NotObject),
    ("trailing-garbage.json", TextReply::NotObject),
    ("invalid-utf8.bin", TextReply::ReplacedText),
    ("empty.bin", TextReply::EmptyText),
];

fn expected<T>(table: &[(&str, T)], name: &str) -> T
where
    T: Copy,
{
    let Some((_, want)) = table.iter().find(|(seed, _)| *seed == name) else {
        panic!("the seed {name} has no expected reply");
    };
    *want
}

const fn failure_of(error: &ReplyError) -> Failure {
    match error {
        ReplyError::Missing { .. } => Failure::Missing,
        ReplyError::Shape { .. } => Failure::Shape,
        ReplyError::Field { .. } => Failure::Field,
        ReplyError::NotObject | ReplyError::TooLarge { .. } | ReplyError::TooManyTokens { .. } => {
            Failure::Other
        }
    }
}

fn status_fields(meta: &TranscriptionMeta) -> (Option<&str>, Option<i64>) {
    (
        meta.status.as_deref(),
        meta.audio_duration_ms.as_ref().and_then(JsonValue::as_i64),
    )
}

fn expect_resource(name: &str, reply: &SonioxReply) {
    let identified = identified_resource(reply);
    let found = identified.as_ref().map_err(failure_of);
    let observed = found.map(|resource| resource.id.as_str());
    assert_eq!(observed, expected(RESOURCE_REPLIES, name));
}

fn expect_transcript(name: &str, reply: &SonioxReply) {
    let decoded = transcript(reply, &ApiLimits::default());
    let found = decoded.as_ref().map_err(failure_of);
    let observed = found.map(|body| (body.text.as_deref(), body.tokens.len()));
    assert_eq!(observed, expected(TRANSCRIPT_REPLIES, name));
}

fn expect_status(name: &str, reply: &SonioxReply) {
    let meta = transcription_meta(reply);
    let found = meta.as_ref().map_err(failure_of);
    let observed = found.map(status_fields);
    assert_eq!(observed, expected(STATUS_REPLIES, name));
}

fn expect_text(name: &str, reply: &SonioxReply) {
    match expected(TEXT_REPLIES, name) {
        TextReply::Message(message) => assert_eq!(provider_message(reply), message),
        TextReply::MessageIsText => assert_eq!(provider_message(reply), reply.text),
        TextReply::NotObject => assert!(reply.payload.is_none()),
        TextReply::NotObjectText(text) => {
            assert!(reply.payload.is_none());
            assert_eq!(reply.text, text);
        }
        TextReply::ReplacedText => {
            assert!(reply.payload.is_none());
            assert!(reply.text.contains('\u{fffd}'));
        }
        TextReply::EmptyText => {
            assert!(reply.payload.is_none());
            assert!(reply.text.is_empty());
        }
    }
}
