#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use indexmap::IndexMap;

use crate::completion::stream::chunk_parser;

fn chunk(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parsed_chunks() {
    let parsed = chunk_parser(&chunk(r#"{"completion":"Hi","stop_reason":null}"#)).unwrap();
    let expected = GenericChunk {
        text: "Hi".to_owned(),
        tool_use: None,
        is_finished: true,
        finish_reason: String::new(),
        usage: None,
        index: Some(0),
        provider_specific_fields: None,
    };
    assert_eq!(parsed, expected);
    let other = chunk(r#"{"completion":5,"index":3,"stop_reason":"max_tokens"}"#);
    let expected_other = GenericChunk {
        text: String::new(),
        tool_use: None,
        is_finished: true,
        finish_reason: "max_tokens".to_owned(),
        usage: None,
        index: Some(3),
        provider_specific_fields: None,
    };
    assert_eq!(chunk_parser(&other).unwrap(), expected_other);
    assert_eq!(chunk_parser(&chunk("{}")).unwrap().text, "");
    let null_text = chunk(r#"{"completion":null}"#);
    assert_eq!(chunk_parser(&null_text).unwrap().text, "");
    for invalid in [
        r#"{"index":"x"}"#,
        r#"{"index":1.5}"#,
        r#"{"stop_reason":5}"#,
    ] {
        assert!(chunk_parser(&chunk(invalid)).is_err(), "{invalid}");
    }
}
