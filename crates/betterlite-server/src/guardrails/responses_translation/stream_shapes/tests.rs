#![cfg(test)]
use super::*;

#[test]
fn envelope_reads_partial_fields_and_ignores_extra() {
    let value: ResponseOutputEnvelope =
        serde_json::from_str(r#"{"model":"m","output":[],"other":1}"#).unwrap();
    assert_eq!(value.model.as_deref(), Some("m"));
    assert_eq!(value.output, Some(Vec::new()));
}

#[test]
fn stream_chunk_reads_optional_fields() {
    let value: ResponsesStreamChunk =
        serde_json::from_str(r#"{"type":"delta","delta":"x","output_index":2}"#).unwrap();
    assert_eq!(value.delta.as_deref(), Some("x"));
    assert_eq!(value.output_index, Some(2));
}
