#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use rstest::rstest;

use super::{
    CursorError, ErrorCursor, decode_error_cursor, decode_trace_cursor, encode_error_cursor,
    encode_trace_cursor,
};

const VALID_VERSION: &str = "A1B2C3D4E5F6071829384756AFBECD1029384756473829102938475647382910";

#[test]
fn decodes_an_absent_trace_cursor_as_the_empty_start() {
    assert_eq!(decode_trace_cursor(None), Ok((0, String::new())));
}

#[test]
fn round_trips_a_trace_cursor() {
    let cursor = encode_trace_cursor(1_700_000_000_000, "trace-abc");
    assert_eq!(
        decode_trace_cursor(Some(&cursor)),
        Ok((1_700_000_000_000, "trace-abc".to_owned()))
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
    assert_eq!(
        decode_trace_cursor(Some(cursor)),
        Err(CursorError::InvalidTrace)
    );
}

#[test]
fn invalid_trace_cursor_reports_the_litellm_message() {
    let error = decode_trace_cursor(Some("not valid base64 at all!!")).unwrap_err();
    assert_eq!(error.to_string(), "Invalid trace cursor");
}

#[test]
fn decodes_an_absent_diagnostic_cursor_as_none() {
    assert_eq!(decode_error_cursor(None), Ok(None));
}

#[test]
fn round_trips_an_error_cursor() {
    let cursor = ErrorCursor {
        offset: 4096,
        version: VALID_VERSION.to_owned(),
    };
    let encoded = encode_error_cursor(&cursor);
    assert_eq!(decode_error_cursor(Some(&encoded)), Ok(Some(cursor)));
}

#[rstest]
#[case::not_base64("not valid base64 at all!!")]
#[case::not_json(&URL_SAFE.encode("not json"))]
#[case::short_version(&URL_SAFE.encode("{\"offset\":0,\"version\":\"AB\"}"))]
#[case::lowercase_version(&URL_SAFE.encode(format!("{{\"offset\":0,\"version\":\"{}\"}}", VALID_VERSION.to_lowercase())))]
#[case::offset_too_large(&URL_SAFE.encode(format!("{{\"offset\":18446744073709551615,\"version\":\"{VALID_VERSION}\"}}")))]
fn rejects_an_invalid_diagnostic_cursor(#[case] cursor: &str) {
    assert_eq!(
        decode_error_cursor(Some(cursor)),
        Err(CursorError::InvalidDiagnostic)
    );
}

#[test]
fn invalid_diagnostic_cursor_reports_the_litellm_message() {
    let error = decode_error_cursor(Some("not valid base64 at all!!")).unwrap_err();
    assert_eq!(error.to_string(), "Invalid diagnostic cursor");
}
