#![cfg(test)]

use super::records::{
    VectorStoreIndexType, VectorStoreIntegration, VectorStoreSearchFailureMode,
    VectorStoreToolParams, WireRecord,
};
use super::wire::{VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records};
use rstest::rstest;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0001_0000,
        max_event_bytes: 0x0001_0000,
        max_items: 4096,
    }
}

fn assert_wire_record(kind: VectorStoreRecordKind, input: &[u8]) {
    let decoded = decode_vector_store_records(kind, input, limits()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::from_slice::<Value>(input).unwrap()
    );
}

fn assert_fixture_record(fixture: &Value, name: &str, kind: VectorStoreRecordKind) {
    let input = serde_json::to_vec(&fixture[name]).unwrap();
    assert_wire_record(kind, &input);
}

#[test]
fn tool_params_omit_only_null() {
    let empty = VectorStoreToolParams::default().to_dict();
    assert_eq!(empty, WireRecord::new());
    let empty_object: Value = serde_json::from_str("{}").unwrap();
    let explicit_null = VectorStoreToolParams {
        filters: Some(Value::Null),
        max_num_results: Some(0),
        ranking_options: Some(empty_object),
    };
    let converted = explicit_null.to_dict();
    assert_eq!(converted.len(), 2);
    assert!(!converted.contains_key("filters"));
}

#[test]
fn management_record_defaults() {
    let input = br#"{"vector_store_id":"vs_a","unknown":1}"#;
    let decoded =
        decode_vector_store_records(VectorStoreRecordKind::UpdateRequest, input, limits()).unwrap();
    let output = serde_json::to_value(decoded).unwrap();
    let expected = serde_json::from_str::<Value>(r#"{"vector_store_id":"vs_a","custom_llm_provider":null,"vector_store_name":null,"vector_store_description":null,"vector_store_metadata":null}"#).unwrap();
    assert_eq!(output, expected);
}

#[rstest]
#[case(VectorStoreRecordKind::DeleteRequest)]
#[case(VectorStoreRecordKind::InfoRequest)]
fn delete_and_info_require_only_id(#[case] kind: VectorStoreRecordKind) {
    decode_vector_store_records(kind, b"{}", limits()).unwrap_err();
    let input = br#"{"vector_store_id":"vs_a","unknown":1}"#;
    let decoded = decode_vector_store_records(kind, input, limits()).unwrap();
    let output = serde_json::to_value(decoded).unwrap();
    let expected = serde_json::from_str::<Value>(r#"{"vector_store_id":"vs_a"}"#).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn index_management_models_apply_defaults_and_literals() {
    let create_input = br#"{"index_name":"notes","litellm_params":{"vector_store_index":"notes","vector_store_name":"Notes"}}"#;
    let create = decode_vector_store_records(
        VectorStoreRecordKind::IndexCreateRequest,
        create_input,
        limits(),
    )
    .unwrap();
    let create_value = serde_json::to_value(create).unwrap();
    assert_eq!(create_value["index_info"], Value::Null);

    let managed_input = br#"{"id":"idx_a","index_name":"notes","litellm_params":{"vector_store_index":"notes","vector_store_name":"Notes"},"created_at":"2025-01-02T03:04:05Z","unknown":1}"#;
    let managed =
        decode_vector_store_records(VectorStoreRecordKind::ManagedIndex, managed_input, limits())
            .unwrap();
    let managed_value = serde_json::to_value(managed).unwrap();
    assert_eq!(managed_value["created_at"], "2025-01-02T03:04:05Z");
    assert_eq!(managed_value["updated_by"], Value::Null);
    assert!(!managed_value.as_object().unwrap().contains_key("unknown"));

    let invalid_date = br#"{"id":"idx_a","index_name":"notes","litellm_params":{"vector_store_index":"notes","vector_store_name":"Notes"},"created_at":"2025-02-30T03:04:05Z"}"#;
    decode_vector_store_records(VectorStoreRecordKind::ManagedIndex, invalid_date, limits())
        .unwrap_err();
    let list_input = br#"{"data":[]}"#;
    let list = decode_vector_store_records(
        VectorStoreRecordKind::IndexListResponse,
        list_input,
        limits(),
    )
    .unwrap();
    let list_value = serde_json::to_value(list).unwrap();
    assert_eq!(list_value["object"], "list");
    decode_vector_store_records(
        VectorStoreRecordKind::IndexListResponse,
        br#"{"object":"other","data":[]}"#,
        limits(),
    )
    .unwrap_err();
}

#[test]
fn typed_dict_records_preserve_absent_null_and_extensions() {
    let input =
        br#"{"query":"q","filters":null,"rewrite_query":false,"provider_extension":{"x":1}}"#;
    let decoded =
        decode_vector_store_records(VectorStoreRecordKind::SearchRequest, input, limits()).unwrap();
    let output = serde_json::to_value(decoded).unwrap();
    let expected = serde_json::from_slice::<Value>(input).unwrap();
    assert_eq!(output, expected);
    assert!(!output.as_object().unwrap().contains_key("ranking_options"));
}

#[test]
fn managed_auth_create_search_and_failure_fields_round_trip() {
    let managed_input = br#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":"notes","vector_store_description":"d","vector_store_metadata":"raw","created_at":"2025-01-02T03:04:05Z","updated_at":"2025-01-02T03:04:05Z","litellm_credential_name":"c","litellm_params":{"api_base":"url"},"team_id":"t","user_id":"u","is_config":true}"#;
    assert_wire_record(VectorStoreRecordKind::ManagedStore, managed_input);

    let auth_input = br#"{"headers":{"authorization":"secret"},"query_params":{"key":"value"}}"#;
    assert_wire_record(VectorStoreRecordKind::IndexEndpoints, auth_input);

    let create_input = br#"{"name":null,"file_ids":["file_a"],"expires_after":{"anchor":"last_active_at","days":3},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"metadata":{"tag":"x"}}"#;
    assert_wire_record(VectorStoreRecordKind::CreateRequest, create_input);

    let failure_input =
        br#"{"vector_store_id":"vs_a","custom_llm_provider":null,"error":"failed"}"#;
    assert_wire_record(VectorStoreRecordKind::SearchFailure, failure_input);
}

#[test]
fn discovery_and_knowledge_base_records_keep_nested_fields() {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    assert_fixture_record(
        &fixture,
        "discovery_data_store",
        VectorStoreRecordKind::DiscoveryDataStore,
    );
    assert_fixture_record(
        &fixture,
        "discovery_engine",
        VectorStoreRecordKind::DiscoveryEngine,
    );
    assert_fixture_record(
        &fixture,
        "search_response",
        VectorStoreRecordKind::SearchResponse,
    );
    assert_fixture_record(
        &fixture,
        "create_response",
        VectorStoreRecordKind::CreateResponse,
    );
    assert_fixture_record(
        &fixture,
        "bedrock_request",
        VectorStoreRecordKind::BedrockRequest,
    );
    assert_fixture_record(
        &fixture,
        "bedrock_response",
        VectorStoreRecordKind::BedrockResponse,
    );
}

#[rstest]
#[case(r#""bedrock""#, VectorStoreIntegration::Bedrock)]
#[case(r#""ragflow""#, VectorStoreIntegration::Ragflow)]
fn integration_variants(#[case] input: &str, #[case] expected: VectorStoreIntegration) {
    assert_eq!(
        serde_json::from_str::<VectorStoreIntegration>(input).unwrap(),
        expected
    );
}

#[rstest]
#[case(r#""read""#, VectorStoreIndexType::Read)]
#[case(r#""write""#, VectorStoreIndexType::Write)]
fn index_type_variants(#[case] input: &str, #[case] expected: VectorStoreIndexType) {
    assert_eq!(
        serde_json::from_str::<VectorStoreIndexType>(input).unwrap(),
        expected
    );
}

#[rstest]
#[case(r#""annotate""#, VectorStoreSearchFailureMode::Annotate)]
#[case(r#""error""#, VectorStoreSearchFailureMode::Error)]
fn failure_mode_variants(#[case] input: &str, #[case] expected: VectorStoreSearchFailureMode) {
    assert_eq!(
        serde_json::from_str::<VectorStoreSearchFailureMode>(input).unwrap(),
        expected
    );
}

#[test]
fn score_numeric_boundary_uses_exact_decimal_range() {
    let accepted = br#"{"data":[{"score":0.125}]}"#;
    decode_vector_store_records(VectorStoreRecordKind::SearchResponse, accepted, limits()).unwrap();
    let too_large = br#"{"data":[{"score":1e100}]}"#;
    decode_vector_store_records(VectorStoreRecordKind::SearchResponse, too_large, limits())
        .unwrap_err();
    let nonfinite = br#"{"data":[{"score":Infinity}]}"#;
    decode_vector_store_records(VectorStoreRecordKind::SearchResponse, nonfinite, limits())
        .unwrap_err();
}

#[test]
fn rejects_unknown_record_kind() {
    VectorStoreRecordKind::try_from(255).unwrap_err();
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vector_store_records");
    fs::read_dir(corpus)
        .expect("corpus")
        .map(|entry| fs::read(entry.expect("entry").path()).expect("seed"))
        .filter_map(|bytes| {
            let (&kind_byte, json) = bytes.split_first()?;
            let record_kind = VectorStoreRecordKind::try_from(kind_byte).ok()?;
            Some(decode_vector_store_records(record_kind, json, limits()))
        })
        .for_each(drop);
}
