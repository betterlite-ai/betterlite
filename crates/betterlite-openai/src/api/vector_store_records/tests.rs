#![cfg(test)]

use super::{
    VectorStoreRecordKind, VectorStoreToolParams, WireDecodeLimits, WireRecord,
    decode_vector_store_records, validate_model,
};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[test]
fn tool_params_omit_only_null() {
    assert_eq!(
        VectorStoreToolParams::default().to_dict(),
        WireRecord::new()
    );
    let empty_object: Value = serde_json::from_str("{}").unwrap();
    let params = VectorStoreToolParams {
        filters: Some(empty_object.clone()),
        max_num_results: Some(0),
        ranking_options: Some(empty_object),
    };
    assert_eq!(params.to_dict().len(), 3);
}

#[test]
fn validates_model_required_fields() {
    let missing = Value::Object(serde_json::Map::new());
    validate_model(VectorStoreRecordKind::UpdateRequest, &missing).unwrap_err();
    let record = serde_json::from_str(r#"{"vector_store_id":"vs_a","extra":1}"#).unwrap();
    validate_model(VectorStoreRecordKind::UpdateRequest, &record).unwrap();
}

#[test]
fn rejects_unknown_record_kind() {
    VectorStoreRecordKind::try_from(255).unwrap_err();
}

#[test]
fn replays_corpus() {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0001_0000,
        max_event_bytes: 0x0001_0000,
        max_items: 4096,
    };
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vector_store_records");
    fs::read_dir(corpus)
        .expect("corpus")
        .map(|entry| fs::read(entry.expect("entry").path()).expect("seed"))
        .filter_map(|bytes| {
            let (&kind_byte, json) = bytes.split_first()?;
            let record_kind = VectorStoreRecordKind::try_from(kind_byte).ok()?;
            Some(decode_vector_store_records(record_kind, json, limits))
        })
        .for_each(drop);
}
