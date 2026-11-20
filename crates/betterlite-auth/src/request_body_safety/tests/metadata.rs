use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::request_body_safety::{
    DecodeError, MAX_METADATA_BYTES, MAX_METADATA_DEPTH, SafetyError, decode_request_metadata,
};

use super::Fixture;

fn metadata_body(value: Json) -> JsonObject {
    let mut body = JsonObject::new();
    body.insert("metadata".to_owned(), value);
    body
}

#[test]
fn metadata_byte_limit() {
    let oversized = format!("\"{}\"", "a".repeat(MAX_METADATA_BYTES));
    assert!(oversized.len() > MAX_METADATA_BYTES);
    assert!(matches!(
        decode_request_metadata(oversized.as_bytes()),
        Err(DecodeError::TooLarge { .. })
    ));
    // exactly at the bound the decoder still parses
    let at_limit = format!("\"{}\"", "a".repeat(MAX_METADATA_BYTES - 2));
    decode_request_metadata(at_limit.as_bytes()).expect("the byte bound itself parses");

    let fixture = Fixture::default();
    let error = fixture
        .check(&metadata_body(Json::Text(oversized)), "model")
        .expect_err("an oversized stringified metadata is refused");
    assert!(matches!(
        error,
        SafetyError::MetadataLimit { ref error } if matches!(**error, DecodeError::TooLarge { .. })
    ));
}

#[test]
fn metadata_depth_limit() {
    let deep = format!(
        "{}1{}",
        "[".repeat(MAX_METADATA_DEPTH + 1),
        "]".repeat(MAX_METADATA_DEPTH + 1)
    );
    assert!(matches!(
        decode_request_metadata(deep.as_bytes()),
        Err(DecodeError::TooDeep { .. })
    ));
    let at_limit = format!(
        "{}1{}",
        "[".repeat(MAX_METADATA_DEPTH),
        "]".repeat(MAX_METADATA_DEPTH)
    );
    decode_request_metadata(at_limit.as_bytes()).expect("the depth bound itself parses");

    // brackets inside a string are text, and an escaped quote does not end the string early
    let quoted = format!(
        r#"{{"value": "a\"b", "note": "{}"}}"#,
        "[".repeat(MAX_METADATA_DEPTH + 1)
    );
    decode_request_metadata(quoted.as_bytes())
        .expect("brackets inside a string are not containers");
    // a string does not hide the container nesting that follows it
    let after_string = format!(
        r#"{{"value": "a\"b", "nested": {}1{}}}"#,
        "[".repeat(MAX_METADATA_DEPTH + 1),
        "]".repeat(MAX_METADATA_DEPTH + 1)
    );
    assert!(matches!(
        decode_request_metadata(after_string.as_bytes()),
        Err(DecodeError::TooDeep { .. })
    ));
    // a doubled backslash is one escaped backslash, so the quote after it still closes the string
    let escaped_backslash = format!(
        r#"{{"value": "a\\", "note": "{}"}}"#,
        "[".repeat(MAX_METADATA_DEPTH + 1)
    );
    decode_request_metadata(escaped_backslash.as_bytes())
        .expect("a doubled backslash ends the string at its closing quote");
    // sibling containers do not accumulate: every closing bracket lowers the depth again
    let siblings = format!("[{}]", ["[]"; MAX_METADATA_DEPTH + 1].join(","));
    decode_request_metadata(siblings.as_bytes()).expect("sibling arrays stay at depth two");
    // scalars between the openings do not hide real nesting
    let interleaved = format!(
        "{}1{}",
        "[1,".repeat(MAX_METADATA_DEPTH + 1),
        "]".repeat(MAX_METADATA_DEPTH + 1)
    );
    assert!(matches!(
        decode_request_metadata(interleaved.as_bytes()),
        Err(DecodeError::TooDeep { .. })
    ));

    let fixture = Fixture::default();
    let error = fixture
        .check(&metadata_body(Json::Text(deep)), "model")
        .expect_err("too-deep stringified metadata is refused");
    assert!(matches!(
        error,
        SafetyError::MetadataLimit { ref error } if matches!(**error, DecodeError::TooDeep { .. })
    ));
}
