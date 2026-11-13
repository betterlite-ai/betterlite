#![cfg(test)]

use aws_smithy_types::event_stream::HeaderValue;
use bytes::Bytes;

use super::frames::{
    bedrock_parts, chunk_frame, error_frame, event, event_frame, frame, kind, text,
};
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;

fn parse(wire: &[u8]) -> Result<Option<String>, &'static str> {
    let shape = ResponseStreamShape::bedrock_runtime();
    parse_message_from_event(Some(&shape), &event(wire)).map_err(|error| kind(&error))
}

fn error_parts(wire: &[u8]) -> (u16, String) {
    let shape = ResponseStreamShape::bedrock_runtime();
    bedrock_parts(parse_message_from_event(Some(&shape), &event(wire)).unwrap_err())
}

pub(super) fn exception_frame() {
    let body = br#"{"message":"bad"}"#;
    let modeled = [
        ("internalServerException", 500),
        ("modelStreamErrorException", 424),
        ("validationException", 400),
        ("throttlingException", 429),
        ("modelTimeoutException", 408),
        ("serviceUnavailableException", 503),
        ("chunk", 400),
        ("somethingElse", 400),
    ];
    for kind_name in ["error", "exception"] {
        for (name, status) in modeled {
            let found = error_parts(&error_frame(kind_name, Some(name), body));
            assert_eq!(
                found,
                (status, format!("{name} {{\"message\":\"bad\"}}")),
                "{name}"
            );
        }
    }
    assert_eq!(parse(&error_frame("error", None, body)), Err("Bedrock"));
    exception_bodies();
}

fn exception_bodies() {
    let plain = (400, "plain".to_owned());
    assert_eq!(
        error_parts(&error_frame("error", Some(""), b"plain")),
        plain
    );
    let numeric = frame(
        &[
            (":message-type", text("error")),
            (":exception-type", HeaderValue::Int32(7)),
        ],
        b"plain",
    );
    assert_eq!(error_parts(&numeric), plain);
    assert_eq!(
        error_parts(&error_frame(
            "error",
            Some("validationException"),
            b"not json"
        )),
        (400, "validationException not json".to_owned())
    );
    for (payload, expected) in [
        (&b"[1]"[..], "PayloadNotObject"),
        (b"null", "PayloadNotObject"),
        (b"\xff", "NotUtf8"),
    ] {
        let wire = error_frame("error", Some("validationException"), payload);
        assert_eq!(parse(&wire), Err(expected), "{payload:?}");
    }
    assert_eq!(
        error_parts(&error_frame("error", Some("other"), b"[1]")),
        (400, "other [1]".to_owned())
    );
    assert_eq!(
        parse(&error_frame("error", Some("other"), b"\xff")),
        Err("NotUtf8")
    );
}

pub(super) fn unknown_frame() {
    assert_eq!(
        parse(&event_frame("mystery", b"raw text")),
        Ok(Some("raw text".to_owned()))
    );
    let no_type = frame(&[(":message-type", text("event"))], b"raw");
    assert_eq!(parse(&no_type), Ok(Some("raw".to_owned())));
    assert_eq!(parse(&event_frame("mystery", b"")), Ok(None));
    assert_eq!(parse(&event_frame("mystery", b"\xff")), Err("NotUtf8"));
    let modeled_non_chunk = event_frame("internalServerException", b"{\"message\":\"x\"}");
    assert_eq!(
        parse(&modeled_non_chunk),
        Ok(Some("{\"message\":\"x\"}".to_owned()))
    );
    let bytes_type = frame(
        &[(
            ":event-type",
            HeaderValue::ByteArray(Bytes::from_static(b"chunk")),
        )],
        b"{\"bytes\":\"aGk=\"}",
    );
    assert_eq!(
        parse(&bytes_type),
        Ok(Some("{\"bytes\":\"aGk=\"}".to_owned()))
    );
    assert_eq!(parse(&chunk_frame("hi")), Ok(Some("hi".to_owned())));
    for payload in [&b"{}"[..], b"{\"bytes\":null}", b""] {
        assert_eq!(
            parse(&event_frame("chunk", payload)),
            Ok(None),
            "{payload:?}"
        );
    }
    assert_eq!(
        parse(&event_frame("chunk", b"{\"bytes\":\"\"}")),
        Ok(Some(String::new()))
    );
}

pub(super) fn json_payload_error() {
    assert_eq!(parse(&event_frame("chunk", b"not json")), Ok(None));
    for payload in [&b"[1]"[..], b"null", b"\"s\"", b"1"] {
        assert_eq!(
            parse(&event_frame("chunk", payload)),
            Err("PayloadNotObject"),
            "{payload:?}"
        );
    }
    for payload in [
        &b"{\"bytes\":5}"[..],
        b"{\"bytes\":[\"a\"]}",
        b"{\"bytes\":{}}",
    ] {
        assert_eq!(
            parse(&event_frame("chunk", payload)),
            Err("ChunkBytesNotText"),
            "{payload:?}"
        );
    }
    assert_eq!(
        parse(&event_frame("chunk", b"{\"bytes\":\"!!!\"}")),
        Err("ChunkBase64")
    );
    assert_eq!(parse(&event_frame("chunk", b"\xff")), Err("NotUtf8"));
    assert_eq!(
        parse(&event_frame("chunk", b"{\"bytes\":\"/w==\"}")),
        Err("NotUtf8")
    );
}

pub(super) fn shape_load_failure() {
    let chunk = event(&chunk_frame("hi"));
    let failure = error_frame("error", Some("validationException"), b"{}");
    let expected = "Bedrock event-stream shape could not be loaded from botocore. Ensure botocore is correctly installed.";
    for found in [chunk, event(&failure)] {
        let failed = parse_message_from_event(None, &found).unwrap_err();
        assert_eq!(bedrock_parts(failed), (500, expected.to_owned()));
    }
    assert_eq!(ResponseStreamShape::bedrock_runtime().members.len(), 7);
}
