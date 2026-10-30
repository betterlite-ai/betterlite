#![cfg(test)]

use std::{fs, str};

use betterlite_error::error::ErrorClass;
use rstest::rstest;
use serde_json::to_string;
use serde_json::value::to_raw_value;

use super::AnthropicExceptionMapping;
use super::decode::{
    DecodedMessagesErrors, WireDecodeError, WireDecodeLimits, decode_messages_errors,
};
use super::errors::AnthropicErrorType;
use super::frame::anthropic_error_sse_frame;
use super::wire::{AnthropicMetadata, ErrorField, ErrorRecord, Missing as _, TextField};

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

#[rstest]
#[case::bad_request(400, "invalid_request_error")]
#[case::authentication(401, "authentication_error")]
#[case::permission(403, "permission_error")]
#[case::not_found(404, "not_found_error")]
#[case::too_large(413, "request_too_large")]
#[case::rate_limit(429, "rate_limit_error")]
#[case::server(500, "api_error")]
#[case::overloaded(529, "overloaded_error")]
#[case::fallback(418, "api_error")]
fn status_error_types(#[case] status: u16, #[case] expected: &str) {
    let response =
        AnthropicExceptionMapping::transform_to_anthropic_error(status, "broken", None, LIMITS)
            .unwrap();
    assert_eq!(
        to_string(&response).unwrap(),
        format!(r#"{{"type":"error","error":{{"type":"{expected}","message":"broken"}}}}"#)
    );
}

#[rstest]
#[case::absent(None, "")]
#[case::empty(Some(""), "")]
#[case::present(Some("req_a"), ",\"request_id\":\"req_a\"")]
fn request_ids(#[case] request_id: Option<&str>, #[case] suffix: &str) {
    let response =
        AnthropicExceptionMapping::transform_to_anthropic_error(500, "broken", request_id, LIMITS)
            .unwrap();
    assert_eq!(
        to_string(&response).unwrap(),
        format!(r#"{{"type":"error","error":{{"type":"api_error","message":"broken"}}{suffix}}}"#)
    );
}

#[rstest]
#[case::detail(
    r#"{"detail":{"message":"inner"},"Message":"aws","message":"generic"}"#,
    Some("\"inner\"")
)]
#[case::duplicates(r#"{"Message":"old","Message":"last"}"#, Some("\"last\""))]
#[case::detail_missing(r#"{"detail":{},"Message":"aws"}"#, None)]
#[case::detail_null(r#"{"detail":{"message":null}}"#, Some("null"))]
#[case::aws(
    r#"{"detail":null,"Message":"aws","message":"generic"}"#,
    Some("\"aws\"")
)]
#[case::generic(r#"{"Message":"","message":"generic"}"#, Some("\"generic\""))]
#[case::empty(r#"{"Message":"","message":""}"#, None)]
#[case::false_values(r#"{"Message":0,"message":false}"#, None)]
#[case::number(r#"{"Message":17}"#, Some("17"))]
#[case::true_value(r#"{"Message":true}"#, Some("true"))]
#[case::zero_exponent(r#"{"Message":-0.0e10,"message":"generic"}"#, Some("\"generic\""))]
#[case::array_value(r#"{"message":[{}]}"#, Some("[{}]"))]
#[case::empty_collections(r#"{"Message":[],"message":{}}"#, None)]
#[case::array("[1]", None)]
#[case::null("null", None)]
#[case::quoted("\"quoted\"", None)]
#[case::malformed("{", None)]
fn provider_message_precedence(#[case] raw: &str, #[case] expected: Option<&str>) {
    let response =
        AnthropicExceptionMapping::transform_to_anthropic_error(500, raw, None, LIMITS).unwrap();
    let ErrorField::Known(detail) = response.head.error else {
        panic!("expected detail")
    };
    assert_eq!(
        to_string(&detail.head.message).unwrap(),
        expected.map_or_else(|| to_string(raw).unwrap(), str::to_owned)
    );
}

#[rstest]
#[case::absent(r#"{"type":"error","error":{"type":"vendor_error","message":"bad","code":"provider"},"extra":7}"#, true)]
#[case::null_id(
    r#"{"type":"error","error":{"type":null,"message":null},"request_id":null}"#,
    true
)]
#[case::old_id(
    r#"{"type":"error","error":{"type":"vendor_error","message":"bad"},"request_id":"req_old"}"#,
    true
)]
#[case::arbitrary(r#"{"type":"error","error":{"type":[],"message":{},"litellm_call_id":null,"provider_specific_fields":null}}"#, true)]
#[case::missing_type(r#"{"type":"error","error":{"message":"bad"}}"#, false)]
#[case::missing_message(r#"{"type":"error","error":{"type":"vendor_error"}}"#, false)]
#[case::array_error(r#"{"type":"error","error":[]}"#, false)]
#[case::other_type(r#"{"type":"other","error":{"type":null,"message":"bad"}}"#, false)]
fn anthropic_error_passthrough(#[case] raw: &str, #[case] passes: bool) {
    let response =
        AnthropicExceptionMapping::transform_to_anthropic_error(401, raw, Some("req_new"), LIMITS)
            .unwrap();
    if passes {
        let mut expected: ErrorRecord = serde_json::from_str(raw).unwrap();
        if expected.head.request_id.is_missing() {
            expected.head.request_id = TextField::Known("req_new".to_owned());
            expected
                .fields
                .insert("request_id".to_owned(), to_raw_value("req_new").unwrap());
        }
        assert_eq!(to_string(&response).unwrap(), to_string(&expected).unwrap());
    } else {
        let ErrorField::Known(detail) = response.head.error else {
            panic!("expected detail")
        };
        assert_eq!(
            to_string(&detail.head.r#type).unwrap(),
            "\"authentication_error\""
        );
        assert_eq!(
            to_string(&detail.head.message).unwrap(),
            to_string(raw).unwrap()
        );
    }
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
#[case::present(Some("call_a"))]
fn status_bearing_error_frame(#[case] call_id: Option<&str>) {
    let frame = anthropic_error_sse_frame(429, "slow", LIMITS).unwrap();
    assert_eq!(frame.status_code, 429);
    let original = r#"{"type": "error", "error": {"type": "rate_limit_error", "message": "slow"}}"#;
    assert_eq!(frame.body, format!("event: error\ndata: {original}\n\n"));
    let body = frame.json_body(call_id).unwrap();
    let ErrorField::Known(detail) = &body.head.error else {
        panic!("expected detail")
    };
    assert_eq!(
        to_string(&detail.head.litellm_call_id).unwrap(),
        to_string(&call_id).unwrap()
    );
    assert_eq!(
        to_string(&frame.error_response).unwrap(),
        original.replace(": ", ":").replace(", ", ",")
    );
}

#[rstest]
#[case::end_turn("\"end_turn\"")]
#[case::max_tokens("\"max_tokens\"")]
#[case::stop_sequence("\"stop_sequence\"")]
#[case::tool_use("\"tool_use\"")]
#[case::refusal("\"refusal\"")]
#[case::compaction("\"compaction\"")]
#[case::null("null")]
fn response_optional_fields(#[case] stop: &str) {
    let template = include_str!("response.json");
    let text = template.trim().replace("\"STOP_REASON\"", stop);
    let result = decode_messages_errors(text.as_bytes(), LIMITS).unwrap();
    assert_eq!(to_string(&result).unwrap(), text);
}

#[rstest]
#[case::empty("{}", r#"{"user_id":null}"#)]
#[case::user(r#"{"user_id":"alice","extra":7}"#, r#"{"user_id":"alice"}"#)]
fn metadata_defaults(#[case] input: &str, #[case] expected: &str) {
    let metadata: AnthropicMetadata = serde_json::from_str(input).unwrap();
    assert_eq!(to_string(&metadata).unwrap(), expected);
}

#[rstest]
#[case::json_bytes(1, 4096, 100)]
#[case::event_bytes(4096, 1, 100)]
#[case::items(4096, 4096, 1)]
fn bounded_inputs(#[case] json: usize, #[case] event: usize, #[case] items: usize) {
    let limits = WireDecodeLimits {
        max_json_bytes: json,
        max_event_bytes: event,
        max_items: items,
    };
    let raw = r#"{ "type" : "message", "nested" : ["a\\\"b", false, null, 17 ] }"#;
    decode_messages_errors(raw.as_bytes(), limits).unwrap_err();
    AnthropicExceptionMapping::transform_to_anthropic_error(500, raw, None, limits).unwrap_err();
    let exact = WireDecodeLimits {
        max_json_bytes: raw.len(),
        max_event_bytes: raw.len(),
        max_items: 7,
    };
    decode_messages_errors(raw.as_bytes(), exact).unwrap();
}

#[test]
fn shared_taxonomy() {
    let class: ErrorClass = serde_json::from_str(
        r#"{"family":"authentication","message":"bad","model":null,"provider":null}"#,
    )
    .unwrap();
    assert_eq!(
        AnthropicErrorType::from(&class.metadata()),
        AnthropicErrorType::AuthenticationError
    );
}

#[test]
fn replays_corpus() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/messages_errors");
    let outcomes: Vec<bool> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            let Some((&selector, payload)) = bytes.split_first() else {
                return false;
            };
            if selector.is_multiple_of(2) {
                decode_messages_errors(payload, LIMITS).is_ok()
            } else if let Ok(raw) = str::from_utf8(payload) {
                AnthropicExceptionMapping::transform_to_anthropic_error(
                    500,
                    raw,
                    Some("fuzz"),
                    LIMITS,
                )
                .is_ok()
            } else {
                false
            }
        })
        .collect();
    assert!(outcomes.contains(&true));
    assert!(outcomes.contains(&false));
    assert!(matches!(
        decode_messages_errors(b"\xff", LIMITS),
        Err(WireDecodeError::Json(_))
    ));
}

#[rstest]
#[case::omitted("{}", false)]
#[case::null_type(r#"{"type":null}"#, false)]
#[case::unknown_type(r#"{"type":"future"}"#, false)]
#[case::error(
    r#"{"type":"error","error":{"type":null,"message":null},"request_id":null}"#,
    true
)]
#[case::wrong_types(
    r#"{"type":"message","model":17,"usage":[],"content":[{"type":"unknown","text":null}]}"#,
    false
)]
fn wire_records(#[case] raw: &str, #[case] error: bool) {
    let record = decode_messages_errors(raw.as_bytes(), LIMITS).unwrap();
    assert_eq!(matches!(record, DecodedMessagesErrors::Error(_)), error);
    assert_eq!(to_string(&record).unwrap(), raw);
}

#[rstest]
#[case::numbers(
    r#"{"type":"error","error":{"type":null,"message":1e-5},"extra":[-0,1.0e0,1e16]}"#,
    r#"{"type": "error", "error": {"type": null, "message": 1e-05}, "extra": [0, 1.0, 1e+16]}"#
)]
#[case::unicode("\u{e9}\u{1f600}\nquote\"", r#"{"type": "error", "error": {"type": "api_error", "message": "\u00e9\ud83d\ude00\nquote\""}}"#)]
#[case::passthrough(r#"{"extra":7,"error":{"message":[{},true,null],"type":"vendor_error"},"type":"error"}"#, r#"{"extra": 7, "error": {"message": [{}, true, null], "type": "vendor_error"}, "type": "error"}"#)]
fn frame_wire_text(#[case] raw: &str, #[case] expected: &str) {
    let frame = anthropic_error_sse_frame(418, raw, LIMITS).unwrap();
    assert_eq!(frame.body, format!("event: error\ndata: {expected}\n\n"));
}

#[test]
fn wire_limits_reject_before_parse() {
    let limits = WireDecodeLimits {
        max_json_bytes: 1,
        max_event_bytes: 1,
        max_items: 100,
    };
    assert!(matches!(
        decode_messages_errors(b"\xff\xff", limits),
        Err(WireDecodeError::TooLarge)
    ));
    assert!(matches!(
        AnthropicExceptionMapping::transform_to_anthropic_error(500, "{{", None, limits),
        Err(WireDecodeError::TooLarge)
    ));
}
