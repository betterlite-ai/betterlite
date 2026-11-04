#![cfg(test)]

use super::records::basic::{
    VectorStoreIndexType, VectorStoreIntegration, VectorStoreSearchFailureMode,
    VectorStoreToolParams, WireRecord,
};
use super::records::bedrock::{BedrockKbRequest, BedrockKbResponse};
use super::records::vector::{
    LiteLlmManagedVectorStore, VectorStoreChunkingStrategy, VectorStoreSearchRequest,
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

    let typed: LiteLlmManagedVectorStore = serde_json::from_str(
        r#"{"vector_store_id":"vs_a","vector_store_name":null,"provider_field":{"enabled":true}}"#,
    )
    .unwrap();
    assert_eq!(typed.vector_store_id.as_deref(), Some("vs_a"));
    assert!(typed.vector_store_name.is_none());
    assert!(typed.extra["vector_store_name"].is_null());
    assert_eq!(typed.extra["provider_field"]["enabled"], true);
    assert!(typed.created_at.is_none());
    assert!(!typed.extra.contains_key("created_at"));
}

#[test]
fn search_records_keep_absent_and_explicit_null_distinct() {
    let explicit_null: VectorStoreSearchRequest =
        serde_json::from_str(r#"{"query":null,"filters":null}"#).unwrap();
    assert!(explicit_null.query.is_none());
    assert!(explicit_null.extra["query"].is_null());
    assert!(explicit_null.extra["filters"].is_null());
    assert_eq!(
        serde_json::to_value(explicit_null).unwrap(),
        serde_json::from_str::<Value>(r#"{"query":null,"filters":null}"#).unwrap()
    );
    let absent: VectorStoreSearchRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(absent).unwrap(),
        serde_json::from_str::<Value>("{}").unwrap()
    );
}

#[test]
fn managed_record_keeps_mistyped_and_extension_fields_raw() {
    let mistyped: LiteLlmManagedVectorStore =
        serde_json::from_str(r#"{"vector_store_id":42}"#).unwrap();
    assert!(mistyped.vector_store_id.is_none());
    assert_eq!(mistyped.extra["vector_store_id"], 42_i32);
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

#[rstest]
#[case::failure_id(
    VectorStoreRecordKind::SearchFailure,
    br#"{"custom_llm_provider":null,"error":"e"}"#
)]
#[case::failure_provider(
    VectorStoreRecordKind::SearchFailure,
    br#"{"vector_store_id":"v","error":"e"}"#
)]
#[case::failure_error(
    VectorStoreRecordKind::SearchFailure,
    br#"{"vector_store_id":"v","custom_llm_provider":null}"#
)]
#[case::bedrock_user_id(VectorStoreRecordKind::BedrockRequest, br#"{"userContext":{}}"#)]
#[case::row_column_name(
    VectorStoreRecordKind::BedrockRow,
    br#"{"columnValue":"v","type":"STRING"}"#
)]
#[case::row_column_value(
    VectorStoreRecordKind::BedrockRow,
    br#"{"columnName":"c","type":"STRING"}"#
)]
#[case::row_type(
    VectorStoreRecordKind::BedrockRow,
    br#"{"columnName":"c","columnValue":"v"}"#
)]
#[case::nested_row(
    VectorStoreRecordKind::BedrockResponse,
    br#"{"retrievalResults":[{"content":{"row":[{"columnValue":"v","type":"STRING"}]}}]}"#
)]
fn required_typed_dict_fields_reject_missing_keys(
    #[case] kind: VectorStoreRecordKind,
    #[case] input: &[u8],
) {
    decode_vector_store_records(kind, input, limits()).unwrap_err();
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

#[test]
fn bedrock_parent_records_decode_typed_nested_fields() {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    let request: BedrockKbRequest =
        serde_json::from_value(fixture["bedrock_request"].clone()).unwrap();
    let retrieval = request.retrieval_configuration.unwrap();
    let vector_search = retrieval.vector_search_configuration.unwrap();
    let reranking = vector_search.reranking_configuration.unwrap();
    let model = reranking.bedrock_rerank_config.unwrap();
    assert_eq!(
        model.model_configuration.unwrap().model_arn.as_deref(),
        Some("a")
    );
    assert_eq!(
        model
            .metadata_configuration
            .unwrap()
            .selection_mode
            .as_deref(),
        Some("SELECTIVE")
    );
    let response: BedrockKbResponse =
        serde_json::from_value(fixture["bedrock_response"].clone()).unwrap();
    let result = &response.retrieval_results.as_ref().unwrap()[0];
    assert_eq!(
        result
            .score
            .as_ref()
            .map(serde_json::Number::to_string)
            .as_deref(),
        Some("0.5")
    );
    assert_eq!(
        result.content.as_ref().unwrap().row.as_ref().unwrap()[0]
            .column_name
            .as_str(),
        "c"
    );
}

#[test]
fn chunking_strategy_decodes_into_named_variants() {
    let auto: VectorStoreChunkingStrategy = serde_json::from_str(r#"{"type":"auto"}"#).unwrap();
    let auto_type = match auto {
        VectorStoreChunkingStrategy::Auto(value) => value.type_name,
        VectorStoreChunkingStrategy::Static(_) | VectorStoreChunkingStrategy::Other(_) => None,
    };
    assert_eq!(auto_type.as_deref(), Some("auto"));
    let fixed: VectorStoreChunkingStrategy = serde_json::from_str(
        r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#,
    )
    .unwrap();
    let static_chunk_size = match fixed {
        VectorStoreChunkingStrategy::Static(value) => value
            .static_config
            .and_then(|config| config.max_chunk_size_tokens),
        VectorStoreChunkingStrategy::Auto(_) | VectorStoreChunkingStrategy::Other(_) => None,
    };
    assert_eq!(static_chunk_size, Some(800));
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
fn every_record_kind_byte_selects_a_record() {
    let kinds = [
        VectorStoreRecordKind::ToolParams,
        VectorStoreRecordKind::ManagedStore,
        VectorStoreRecordKind::ManagedStoreList,
        VectorStoreRecordKind::UpdateRequest,
        VectorStoreRecordKind::DeleteRequest,
        VectorStoreRecordKind::InfoRequest,
        VectorStoreRecordKind::SearchRequest,
        VectorStoreRecordKind::SearchResponse,
        VectorStoreRecordKind::SearchFailure,
        VectorStoreRecordKind::DiscoveryDataStore,
        VectorStoreRecordKind::DiscoveryEngine,
        VectorStoreRecordKind::CreateRequest,
        VectorStoreRecordKind::CreateResponse,
        VectorStoreRecordKind::IndexCreateRequest,
        VectorStoreRecordKind::ManagedIndex,
        VectorStoreRecordKind::IndexListResponse,
        VectorStoreRecordKind::IndexEndpoints,
        VectorStoreRecordKind::BedrockRequest,
        VectorStoreRecordKind::BedrockResponse,
        VectorStoreRecordKind::BedrockRow,
    ];
    kinds
        .into_iter()
        .zip(0_u8..20)
        .for_each(|(expected, byte)| {
            assert_eq!(VectorStoreRecordKind::try_from(byte).unwrap(), expected);
        });
}

#[test]
fn decoder_enforces_each_byte_and_item_limit() {
    let input = br#"{"x":1}"#;
    let mut exact = limits();
    exact.max_json_bytes = input.len();
    exact.max_event_bytes = input.len();
    decode_vector_store_records(VectorStoreRecordKind::ToolParams, input, exact).unwrap();
    let mut json_limit = exact;
    json_limit.max_json_bytes -= 1;
    decode_vector_store_records(VectorStoreRecordKind::ToolParams, input, json_limit).unwrap_err();
    let mut event_limit = exact;
    event_limit.max_event_bytes -= 1;
    decode_vector_store_records(VectorStoreRecordKind::ToolParams, input, event_limit).unwrap_err();
    let mut item_limit = exact;
    item_limit.max_items = 0;
    decode_vector_store_records(VectorStoreRecordKind::ToolParams, input, item_limit).unwrap_err();
    decode_vector_store_records(VectorStoreRecordKind::ToolParams, b"[1]", item_limit).unwrap_err();
}

#[test]
fn decoder_enforces_nesting_depth_boundary() {
    let at_limit = format!("{}0{}", "[".repeat(128), "]".repeat(128));
    let over_limit = format!("{}0{}", "[".repeat(129), "]".repeat(129));
    let boundary = decode_vector_store_records(
        VectorStoreRecordKind::ToolParams,
        at_limit.as_bytes(),
        limits(),
    )
    .unwrap_err();
    assert!(!matches!(
        boundary,
        super::wire::VectorStoreRecordError::TooDeep
    ));
    assert!(matches!(
        decode_vector_store_records(
            VectorStoreRecordKind::ToolParams,
            over_limit.as_bytes(),
            limits()
        ),
        Err(super::wire::VectorStoreRecordError::TooDeep)
    ));
}

#[rstest]
#[case::leap_day("2024-02-29T23:59:59Z", true)]
#[case::century_leap_day("2000-02-29T00:00:00Z", true)]
#[case::non_leap_century("2100-02-29T00:00:00Z", false)]
#[case::non_leap_day("2023-02-29T00:00:00Z", false)]
#[case::thirty_day_overflow("2024-04-31T00:00:00Z", false)]
#[case::thirty_day_end_april("2024-04-30T00:00:00Z", true)]
#[case::thirty_day_end_june("2024-06-30T00:00:00Z", true)]
#[case::thirty_day_end_september("2024-09-30T00:00:00Z", true)]
#[case::thirty_day_end_november("2024-11-30T00:00:00Z", true)]
#[case::leap_february_end("2024-02-28T00:00:00Z", true)]
#[case::common_february_end("2023-02-28T00:00:00Z", true)]
#[case::zero_day("2024-01-00T00:00:00Z", false)]
#[case::hour_overflow("2024-01-01T24:00:00Z", false)]
#[case::minute_overflow("2024-01-01T00:60:00Z", false)]
#[case::second_overflow("2024-01-01T00:00:60Z", false)]
fn managed_index_timestamp_calendar_and_clock_boundaries(
    #[case] timestamp: &str,
    #[case] valid: bool,
) {
    let input = format!(
        "{{\"id\":\"i\",\"index_name\":\"n\",\"litellm_params\":{{\"vector_store_index\":\"n\",\"vector_store_name\":\"n\"}},\"created_at\":\"{timestamp}\"}}"
    );
    assert_eq!(
        decode_vector_store_records(
            VectorStoreRecordKind::ManagedIndex,
            input.as_bytes(),
            limits()
        )
        .is_ok(),
        valid,
        "{timestamp}"
    );
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
