#![cfg(test)]
use super::{ResponseOutputEnvelope, ResponsesStreamChunk};

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

#[test]
fn envelope_absent_and_null_fields_are_none() {
    let empty: ResponseOutputEnvelope = serde_json::from_str("{}").unwrap();
    let nulls: ResponseOutputEnvelope =
        serde_json::from_str(r#"{"output":null,"model":null}"#).unwrap();
    assert_eq!(empty, ResponseOutputEnvelope::default());
    assert_eq!(nulls, empty);
}

#[test]
fn stream_chunk_reads_all_fields_and_preserves_zero_and_empty() {
    let value: ResponsesStreamChunk = serde_json::from_str(r#"{"type":"event","text":"","delta":"delta","item_id":"id","output_index":0,"content_index":3,"other":true}"#).unwrap();
    assert_eq!(
        value,
        ResponsesStreamChunk {
            r#type: Some("event".to_owned()),
            text: Some(String::new()),
            delta: Some("delta".to_owned()),
            item_id: Some("id".to_owned()),
            output_index: Some(0),
            content_index: Some(3),
        }
    );
    let empty: ResponsesStreamChunk = serde_json::from_str("{}").unwrap();
    let nulls: ResponsesStreamChunk = serde_json::from_str(r#"{"type":null,"text":null,"delta":null,"item_id":null,"output_index":null,"content_index":null}"#).unwrap();
    assert_eq!(empty, ResponsesStreamChunk::default());
    assert_eq!(nulls, empty);
}

#[test]
fn stream_chunk_rejects_wrong_declared_field_type() {
    assert!(serde_json::from_str::<ResponsesStreamChunk>(r#"{"output_index":"2"}"#).is_err());
    assert!(serde_json::from_str::<ResponseOutputEnvelope>(r#"{"output":{}}"#).is_err());
}
