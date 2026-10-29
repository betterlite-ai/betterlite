#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use rstest::rstest;
use std::fs;

use super::{
    CursorError, ErrorCursor, decode_error_cursor, decode_trace_cursor, encode_trace_cursor,
};

const VALID_VERSION: &str = "A1B2C3D4E5F6071829384756AFBECD1029384756473829102938475647382910";

#[test]
fn decodes_an_absent_trace_cursor_as_the_empty_start() {
    assert_eq!(decode_trace_cursor(None).expect("ok"), (0, String::new()));
}

#[test]
fn round_trips_a_trace_cursor() {
    let cursor = encode_trace_cursor(1_700_000_000_000, "trace-abc");
    assert_eq!(
        decode_trace_cursor(Some(&cursor)).expect("ok"),
        (1_700_000_000_000, "trace-abc".to_owned())
    );
}

#[rstest]
#[case::not_base64("not valid base64 at all!!")]
#[case::not_json(&URL_SAFE.encode("not json"))]
#[case::not_an_array(&URL_SAFE.encode("{\"a\":1}"))]
#[case::wrong_length(&URL_SAFE.encode("[1]"))]
#[case::zero_start(&URL_SAFE.encode("[0,\"trace-id\"]"))]
#[case::negative_start(&URL_SAFE.encode("[-1,\"trace-id\"]"))]
#[case::boolean_start(&URL_SAFE.encode("[true,\"trace-id\"]"))]
#[case::empty_trace_id(&URL_SAFE.encode("[5,\"\"]"))]
#[case::numeric_trace_id(&URL_SAFE.encode("[5,6]"))]
fn rejects_an_invalid_trace_cursor(#[case] cursor: &str) {
    assert!(matches!(
        decode_trace_cursor(Some(cursor)),
        Err(CursorError::InvalidTrace(_))
    ));
}

#[test]
fn invalid_trace_cursor_reports_the_litellm_message() {
    let error = decode_trace_cursor(Some("not valid base64 at all!!")).unwrap_err();
    assert_eq!(error.to_string(), "Invalid trace cursor");
}

#[test]
fn decodes_an_absent_diagnostic_cursor_as_none() {
    assert_eq!(decode_error_cursor(None).expect("ok"), None);
}

#[test]
fn round_trips_an_error_cursor() {
    let payload = ErrorCursor {
        offset: 4096,
        version: VALID_VERSION.to_owned(),
    };
    let encoded = URL_SAFE.encode(serde_json::to_string(&payload).expect("json"));
    let decoded = decode_error_cursor(Some(&encoded))
        .expect("ok")
        .expect("some");
    assert_eq!(decoded.offset, 4096);
    assert_eq!(decoded.version, VALID_VERSION);
}

#[rstest]
#[case::not_base64("not valid base64 at all!!")]
#[case::not_json(&URL_SAFE.encode("not json"))]
#[case::short_version(&URL_SAFE.encode("{\"offset\":0,\"version\":\"AB\"}"))]
#[case::lowercase_version(&URL_SAFE.encode(format!("{{\"offset\":0,\"version\":\"{}\"}}", VALID_VERSION.to_lowercase())))]
#[case::offset_too_large(&URL_SAFE.encode(format!("{{\"offset\":18446744073709551615,\"version\":\"{VALID_VERSION}\"}}")))]
fn rejects_an_invalid_diagnostic_cursor(#[case] cursor: &str) {
    assert!(matches!(
        decode_error_cursor(Some(cursor)),
        Err(CursorError::InvalidDiagnostic(_))
    ));
}

#[test]
fn invalid_diagnostic_cursor_reports_the_litellm_message() {
    let error = decode_error_cursor(Some("not valid base64 at all!!")).unwrap_err();
    assert_eq!(error.to_string(), "Invalid diagnostic cursor");
}

#[test]
fn replays_the_trace_store_corpus() {
    let corpus_path = format!("{}/fuzz/corpus/cursors", env!("CARGO_MANIFEST_DIR"));
    fs::read_dir(corpus_path)
        .expect("corpus directory")
        .map(|entry| {
            let path = entry.expect("corpus entry").path();
            fs::read_to_string(path).expect("corpus file")
        })
        .for_each(|input| {
            drop(decode_trace_cursor(Some(&input)));
            drop(decode_error_cursor(Some(&input)));
        });
}
