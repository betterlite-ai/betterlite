#![cfg(test)]

use super::records::basic::{
    GuardrailAction, VectorStoreIndexType, VectorStoreIntegration, VectorStoreOpenAiParam,
    VectorStoreSearchFailureMode, VectorStoreStatus, VectorStoreToolParams, WireRecord,
};
use super::records::bedrock::{BedrockKbRequest, BedrockKbResponse};
use super::records::vector::{
    BaseVectorStoreAuthCredentials, LiteLlmManagedVectorStore, LiteLlmVectorStoreConfig,
    ManagedVectorStoreList, VectorStoreChunkingStrategy, VectorStoreCreateResponse,
    VectorStoreFileCounts, VectorStoreIndexEndpoints, VectorStoreSearchRequest,
};
use super::wire::{VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records};
use rstest::rstest;
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error as _;
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
    assert_eq!(converted.get("max_num_results"), Some(&Value::from(0_i64)));
    assert_eq!(
        converted.get("ranking_options"),
        Some(&Value::Object(serde_json::Map::new()))
    );
    assert!(!converted.contains_key("filters"));
    let all_values = VectorStoreToolParams {
        filters: Some(serde_json::json!({})),
        max_num_results: Some(0),
        ranking_options: Some(serde_json::json!({})),
    }
    .to_dict();
    assert_eq!(
        serde_json::to_value(all_values).unwrap(),
        serde_json::from_str::<Value>(r#"{"filters":{},"max_num_results":0,"ranking_options":{}}"#)
            .unwrap()
    );
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
#[case::delete(VectorStoreRecordKind::DeleteRequest)]
#[case::info(VectorStoreRecordKind::InfoRequest)]
fn delete_and_info_require_only_id(#[case] kind: VectorStoreRecordKind) {
    let error = decode_vector_store_records(kind, b"{}", limits()).unwrap_err();
    assert!(
        matches!(&error, super::wire::VectorStoreRecordError::MissingField(field) if field == "vector_store_id")
    );
    assert_eq!(error.details().error_type, Some("missing"));
    assert_eq!(error.details().location, Some("vector_store_id"));
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
    let literal_error = decode_vector_store_records(
        VectorStoreRecordKind::IndexListResponse,
        br#"{"object":"other","data":[]}"#,
        limits(),
    )
    .unwrap_err();
    assert!(
        matches!(&literal_error, super::wire::VectorStoreRecordError::InvalidLiteral { field, .. } if field == "object")
    );
    assert_eq!(literal_error.details().error_type, Some("literal_error"));
    assert_eq!(literal_error.details().location, Some("object"));
}

#[test]
fn index_management_rejects_invalid_nested_values() {
    let nested_bad_date = br#"{"data":[{"id":"i","index_name":"n","litellm_params":{"vector_store_index":"n","vector_store_name":"n"},"created_at":"garbage"}]}"#;
    assert!(matches!(
        decode_vector_store_records(
            VectorStoreRecordKind::IndexListResponse,
            nested_bad_date,
            limits()
        ),
        Err(super::wire::VectorStoreRecordError::Json(_))
    ));
    let bad_index_info = br#"{"index_name":"n","litellm_params":{"vector_store_index":"n","vector_store_name":"n"},"index_info":"x"}"#;
    decode_vector_store_records(
        VectorStoreRecordKind::IndexCreateRequest,
        bad_index_info,
        limits(),
    )
    .unwrap_err();
}

#[rstest]
#[case::fraction("2025-01-02T03:04:05.123Z", "2025-01-02T03:04:05.123000Z")]
#[case::utc_offset("2025-01-02T03:04:05+00:00", "2025-01-02T03:04:05Z")]
#[case::space_separator("2025-01-02 03:04:05Z", "2025-01-02T03:04:05Z")]
#[case::epoch("0", "1970-01-01T00:00:00Z")]
#[case::offset("2025-01-02T03:04:05+02:00", "2025-01-02T03:04:05+02:00")]
#[case::naive("2025-01-02T03:04:05", "2025-01-02T03:04:05")]
#[case::date("2025-01-02", "2025-01-02T00:00:00")]
fn managed_index_timestamps_normalize(#[case] input_timestamp: &str, #[case] expected: &str) {
    let record = format!(
        r#"{{"id":"i","index_name":"n","litellm_params":{{"vector_store_index":"n","vector_store_name":"n"}},"created_at":"{input_timestamp}"}}"#
    );
    let decoded = decode_vector_store_records(
        VectorStoreRecordKind::ManagedIndex,
        record.as_bytes(),
        limits(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap()["created_at"],
        expected
    );
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
fn vector_records_round_trip() {
    let auth_input = br#"{"headers":{"authorization":"secret"},"query_params":{"key":"value"}}"#;
    let auth: BaseVectorStoreAuthCredentials = serde_json::from_slice(auth_input).unwrap();
    assert_eq!(auth.headers.as_ref().unwrap()["authorization"], "secret");
    assert_eq!(auth.query_params.as_ref().unwrap()["key"], "value");
    assert!(auth.extra.is_empty());
    let endpoints: VectorStoreIndexEndpoints = serde_json::from_str(r#"{"read":[["GET","/index"],["POST","/index"]],"write":[["PUT","/index"],["DELETE","/index"],["PATCH","/index"]]}"#).unwrap();
    assert_eq!(endpoints.read.as_ref().unwrap().len(), 2);
    assert_eq!(endpoints.write.as_ref().unwrap().len(), 3);
    let config: LiteLlmVectorStoreConfig =
        serde_json::from_str(r#"{"vector_store_name":"notes","litellm_params":{}}"#).unwrap();
    assert_eq!(config.vector_store_name.as_deref(), Some("notes"));
    assert_eq!(config.litellm_params, Some(serde_json::json!({})));
    assert!(config.extra.is_empty());
    let list: ManagedVectorStoreList = serde_json::from_str(
        r#"{"object":"list","data":[],"total_count":1,"current_page":1,"total_pages":1}"#,
    )
    .unwrap();
    assert_eq!(list.object.as_deref(), Some("list"));
    assert!(list.data.as_ref().unwrap().is_empty());
    assert_eq!(list.total_count, Some(1));
    assert_eq!(list.current_page, Some(1));
    assert_eq!(list.total_pages, Some(1));
    assert!(list.extra.is_empty());
    assert_wire_record(
        VectorStoreRecordKind::IndexEndpoints,
        br#"{"read":[["GET","/index"]],"write":[]}"#,
    );
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

#[rstest]
#[case::search("search_response", VectorStoreRecordKind::SearchResponse)]
#[case::create("create_response", VectorStoreRecordKind::CreateResponse)]
fn create_and_search_records(#[case] name: &str, #[case] kind: VectorStoreRecordKind) {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    assert_fixture_record(&fixture, name, kind);
}

#[test]
fn discovery_extra_body_fields() {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    let store: super::records::vector::VertexSearchDataStoreExtraBody =
        serde_json::from_value(fixture["discovery_data_store"].clone()).unwrap();
    assert!(store.extra.is_empty());
    assert_eq!(store.safe_search, Some(false));
    let engine: super::records::vector::VertexSearchEngineExtraBody =
        serde_json::from_value(fixture["discovery_engine"].clone()).unwrap();
    assert_eq!(engine.num_results_per_data_store, Some(2));
    assert_eq!(
        engine.data_store_specs.as_ref().unwrap()[0]["dataStore"],
        "s"
    );
    assert_eq!(engine.extra["servingConfig"], "provider-extension");
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
}

#[test]
fn knowledge_base_records() {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
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
}

#[test]
fn knowledge_base_response_fields() {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    let response: BedrockKbResponse =
        serde_json::from_value(fixture["bedrock_response"].clone()).unwrap();
    let result = &response.retrieval_results.as_ref().unwrap()[0];
    assert_eq!(
        result
            .score
            .as_ref()
            .map(|score| score.0.to_string())
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

#[rstest]
#[case::auto(r#"{"type":"auto"}"#, "auto", None)]
#[case::static_config(
    r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#,
    "static",
    Some(800)
)]
#[case::static_omitted(r#"{"type":"static"}"#, "static", None)]
fn chunking_strategy_decodes_into_named_variants(
    #[case] input: &str,
    #[case] tag: &str,
    #[case] size: Option<i64>,
) {
    let strategy: VectorStoreChunkingStrategy = serde_json::from_str(input).unwrap();
    let (wire_tag, max_size) = match strategy {
        VectorStoreChunkingStrategy::Auto(value) => (value.type_name, None),
        VectorStoreChunkingStrategy::Static(value) => (
            value.type_name,
            value
                .static_config
                .and_then(|config| config.max_chunk_size_tokens),
        ),
        VectorStoreChunkingStrategy::Other(_) => (None, None),
    };
    assert_eq!(wire_tag.as_deref(), Some(tag));
    assert_eq!(max_size, size);
}

#[rstest]
#[case::bedrock(r#""bedrock""#, VectorStoreIntegration::Bedrock)]
#[case::ragflow(r#""ragflow""#, VectorStoreIntegration::Ragflow)]
fn integration_variants(#[case] input: &str, #[case] expected: VectorStoreIntegration) {
    assert_eq!(
        serde_json::from_str::<VectorStoreIntegration>(input).unwrap(),
        expected
    );
}

#[rstest]
#[case::read(r#""read""#, VectorStoreIndexType::Read)]
#[case::write(r#""write""#, VectorStoreIndexType::Write)]
fn index_type_variants(#[case] input: &str, #[case] expected: VectorStoreIndexType) {
    assert_eq!(
        serde_json::from_str::<VectorStoreIndexType>(input).unwrap(),
        expected
    );
}

#[rstest]
#[case::annotate(r#""annotate""#, VectorStoreSearchFailureMode::Annotate)]
#[case::error(r#""error""#, VectorStoreSearchFailureMode::Error)]
fn failure_mode_variants(#[case] input: &str, #[case] expected: VectorStoreSearchFailureMode) {
    assert_eq!(
        serde_json::from_str::<VectorStoreSearchFailureMode>(input).unwrap(),
        expected
    );
}

#[test]
fn score_numeric_boundary() {
    let accepted = br#"{"data":[{"score":0.125}]}"#;
    decode_vector_store_records(VectorStoreRecordKind::SearchResponse, accepted, limits()).unwrap();
    let too_large = br#"{"data":[{"score":1e100}]}"#;
    let error =
        decode_vector_store_records(VectorStoreRecordKind::SearchResponse, too_large, limits())
            .unwrap_err();
    assert!(matches!(
        error,
        super::wire::VectorStoreRecordError::ScoreOutOfRange(_)
    ));
    assert_eq!(error.details().status, Some(502));
    assert_eq!(error.details().error_type, Some("bad_gateway"));
    assert_eq!(error.details().param, Some("score"));
    assert_eq!(error.details().code, Some("decimal_out_of_range"));
    let beyond_numeric_range = br#"{"data":[{"score":1e999}] }"#;
    assert!(matches!(
        decode_vector_store_records(
            VectorStoreRecordKind::SearchResponse,
            beyond_numeric_range,
            limits()
        ),
        Err(super::wire::VectorStoreRecordError::ScoreOutOfRange(_))
    ));
}

#[test]
fn rejects_unknown_record_kind() {
    VectorStoreRecordKind::try_from(255).unwrap_err();
}

#[rstest]
#[case::tool_params(0, VectorStoreRecordKind::ToolParams)]
#[case::managed_store(1, VectorStoreRecordKind::ManagedStore)]
#[case::managed_store_list(2, VectorStoreRecordKind::ManagedStoreList)]
#[case::update_request(3, VectorStoreRecordKind::UpdateRequest)]
#[case::delete_request(4, VectorStoreRecordKind::DeleteRequest)]
#[case::info_request(5, VectorStoreRecordKind::InfoRequest)]
#[case::search_request(6, VectorStoreRecordKind::SearchRequest)]
#[case::search_response(7, VectorStoreRecordKind::SearchResponse)]
#[case::search_failure(8, VectorStoreRecordKind::SearchFailure)]
#[case::discovery_data_store(9, VectorStoreRecordKind::DiscoveryDataStore)]
#[case::discovery_engine(10, VectorStoreRecordKind::DiscoveryEngine)]
#[case::create_request(11, VectorStoreRecordKind::CreateRequest)]
#[case::create_response(12, VectorStoreRecordKind::CreateResponse)]
#[case::index_create_request(13, VectorStoreRecordKind::IndexCreateRequest)]
#[case::managed_index(14, VectorStoreRecordKind::ManagedIndex)]
#[case::index_list_response(15, VectorStoreRecordKind::IndexListResponse)]
#[case::index_endpoints(16, VectorStoreRecordKind::IndexEndpoints)]
#[case::bedrock_request(17, VectorStoreRecordKind::BedrockRequest)]
#[case::bedrock_response(18, VectorStoreRecordKind::BedrockResponse)]
#[case::bedrock_row(19, VectorStoreRecordKind::BedrockRow)]
fn every_record_kind_byte_selects_a_record(#[case] byte: u8, #[case] kind: VectorStoreRecordKind) {
    assert_eq!(VectorStoreRecordKind::try_from(byte).unwrap(), kind);
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

#[test]
fn index_timestamp_validates_direct_deserialization() {
    let invalid = r#"{"id":"i","index_name":"n","litellm_params":{"vector_store_index":"n","vector_store_name":"n"},"created_at":"garbage"}"#;
    serde_json::from_str::<super::records::pydantic::LiteLlmManagedVectorStoreIndex>(invalid)
        .unwrap_err();
}

#[rstest]
#[case::string_query(br#"{"query":"q","rewrite_query":false}"#)]
#[case::list_query(br#"{"query":["q","r"],"filters":null}"#)]
fn search_query_variants(#[case] input: &[u8]) {
    assert_wire_record(VectorStoreRecordKind::SearchRequest, input);
}

#[rstest]
#[case::expired("expired", VectorStoreStatus::Expired)]
#[case::in_progress("in_progress", VectorStoreStatus::InProgress)]
#[case::completed("completed", VectorStoreStatus::Completed)]
fn create_status_variants(#[case] status: &str, #[case] expected: VectorStoreStatus) {
    let input = format!(r#"{{"status":"{status}","bytes":-1}}"#);
    assert_wire_record(VectorStoreRecordKind::CreateResponse, input.as_bytes());
    let typed: VectorStoreCreateResponse = serde_json::from_str(&input).unwrap();
    assert_eq!(typed.status, Some(expected));
    assert!(typed.extra.is_empty());
}

#[rstest]
#[case::intervened("INTERVENED", GuardrailAction::Intervened)]
#[case::none("NONE", GuardrailAction::None)]
fn guardrail_action_variants(#[case] action: &str, #[case] expected: GuardrailAction) {
    let input = format!(r#"{{"guardrailAction":"{action}"}}"#);
    assert_wire_record(VectorStoreRecordKind::BedrockResponse, input.as_bytes());
    let typed: BedrockKbResponse = serde_json::from_str(&input).unwrap();
    assert_eq!(typed.guardrail_action, Some(expected));
    assert!(typed.extra.is_empty());
}

#[rstest]
#[case::filters("filters", VectorStoreOpenAiParam::Filters)]
#[case::limit("max_num_results", VectorStoreOpenAiParam::MaxNumResults)]
#[case::ranking("ranking_options", VectorStoreOpenAiParam::RankingOptions)]
#[case::rewrite("rewrite_query", VectorStoreOpenAiParam::RewriteQuery)]
fn search_optional_parameter_variants(
    #[case] name: &str,
    #[case] expected: VectorStoreOpenAiParam,
) {
    assert_eq!(
        serde_json::from_value::<VectorStoreOpenAiParam>(Value::String(name.to_owned())).unwrap(),
        expected
    );
}

#[rstest]
#[case::request("bedrock_request", "", VectorStoreRecordKind::BedrockRequest)]
#[case::query(
    "bedrock_request",
    "/retrievalQuery",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::guardrail(
    "bedrock_request",
    "/guardrailConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::retrieval(
    "bedrock_request",
    "/retrievalConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::vector(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::implicit(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration/implicitFilterConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::attribute(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration/implicitFilterConfiguration/metadataAttributes/0",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::reranking(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration/rerankingConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::model(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration/rerankingConfiguration/bedrockRerankingConfiguration/modelConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::metadata(
    "bedrock_request",
    "/retrievalConfiguration/vectorSearchConfiguration/rerankingConfiguration/bedrockRerankingConfiguration/metadataConfiguration",
    VectorStoreRecordKind::BedrockRequest
)]
#[case::response("bedrock_response", "", VectorStoreRecordKind::BedrockResponse)]
#[case::result(
    "bedrock_response",
    "/retrievalResults/0",
    VectorStoreRecordKind::BedrockResponse
)]
#[case::content(
    "bedrock_response",
    "/retrievalResults/0/content",
    VectorStoreRecordKind::BedrockResponse
)]
#[case::location(
    "bedrock_response",
    "/retrievalResults/0/location",
    VectorStoreRecordKind::BedrockResponse
)]
#[case::search("search_response", "/data/0", VectorStoreRecordKind::SearchResponse)]
fn optional_fields_keep_null_and_absence(
    #[case] name: &str,
    #[case] pointer: &str,
    #[case] kind: VectorStoreRecordKind,
) {
    let fixture: Value =
        serde_json::from_slice(include_bytes!("tests/fixtures/wire-records.json")).unwrap();
    let mut input = fixture[name].clone();
    input
        .pointer_mut(pointer)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .values_mut()
        .for_each(|field| *field = Value::Null);
    assert_wire_record(kind, &serde_json::to_vec(&input).unwrap());
    input
        .pointer_mut(pointer)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .clear();
    assert_wire_record(kind, &serde_json::to_vec(&input).unwrap());
}

#[rstest]
#[case::bedrock_overflow(
    VectorStoreRecordKind::BedrockResponse,
    br#"{"retrievalResults":[{"score":1e100}]}"#
)]
#[case::search_underflow(
    VectorStoreRecordKind::SearchResponse,
    br#"{"data":[{"score":1e-29}]}"#
)]
#[case::bedrock_underflow(
    VectorStoreRecordKind::BedrockResponse,
    br#"{"retrievalResults":[{"score":1e-29}]}"#
)]
#[case::search_nonfinite_range(
    VectorStoreRecordKind::SearchResponse,
    br#"{"data":[{"score":1e999}]}"#
)]
fn scores_reject_unrepresentable_decimals(
    #[case] kind: VectorStoreRecordKind,
    #[case] input: &[u8],
) {
    let error = decode_vector_store_records(kind, input, limits()).unwrap_err();
    assert!(matches!(
        error,
        super::wire::VectorStoreRecordError::ScoreOutOfRange(_)
    ));
    assert_eq!(
        error.details(),
        super::wire::ErrorDetails {
            status: Some(502),
            error_type: Some("bad_gateway"),
            param: Some("score"),
            code: Some("decimal_out_of_range"),
            location: None,
        }
    );
    assert!(error.source().is_some());
}

#[rstest]
#[case::index_name(br#"{"litellm_params":{"vector_store_index":"i","vector_store_name":"n"}}"#)]
#[case::parameters(br#"{"index_name":"i"}"#)]
#[case::nested_index(br#"{"index_name":"i","litellm_params":{"vector_store_name":"n"}}"#)]
#[case::nested_name(br#"{"index_name":"i","litellm_params":{"vector_store_index":"i"}}"#)]
fn index_create_requires_declared_fields(#[case] input: &[u8]) {
    decode_vector_store_records(VectorStoreRecordKind::IndexCreateRequest, input, limits())
        .unwrap_err();
}

#[test]
fn update_missing_id_has_validation_details() {
    let error = decode_vector_store_records(VectorStoreRecordKind::UpdateRequest, b"{}", limits())
        .unwrap_err();
    assert_eq!(error.to_string(), "Field required");
    assert_eq!(
        error.details(),
        super::wire::ErrorDetails {
            status: None,
            error_type: Some("missing"),
            param: None,
            code: None,
            location: Some("vector_store_id"),
        }
    );
}

#[test]
fn malformed_json_has_no_provider_error_details() {
    let error = decode_vector_store_records(VectorStoreRecordKind::SearchResponse, b"{", limits())
        .unwrap_err();
    assert!(matches!(
        error,
        super::wire::VectorStoreRecordError::Json(_)
    ));
    assert_eq!(
        error.details(),
        super::wire::ErrorDetails {
            status: None,
            error_type: None,
            param: None,
            code: None,
            location: None,
        }
    );
}

#[test]
fn managed_create_failure_records() {
    let managed_input = br#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":"notes","vector_store_description":"d","vector_store_metadata":"raw","created_at":"2025-01-02T03:04:05Z","updated_at":"2025-01-02T03:04:05Z","litellm_credential_name":"c","litellm_params":{"api_base":"url"},"team_id":"t","user_id":"u","is_config":true}"#;
    assert_wire_record(VectorStoreRecordKind::ManagedStore, managed_input);

    let create_input = br#"{"name":null,"file_ids":["file_a"],"expires_after":{"anchor":"last_active_at","days":3},"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"metadata":{"tag":"x"}}"#;
    assert_wire_record(VectorStoreRecordKind::CreateRequest, create_input);

    let failure_input =
        br#"{"vector_store_id":"vs_a","custom_llm_provider":null,"error":"failed"}"#;
    assert_wire_record(VectorStoreRecordKind::SearchFailure, failure_input);
}

#[test]
fn config_null_keeps_presence() {
    let input: Value =
        serde_json::from_str(r#"{"vector_store_name":"notes","litellm_params":null}"#).unwrap();
    let config: LiteLlmVectorStoreConfig = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(config.vector_store_name.as_deref(), Some("notes"));
    assert!(config.litellm_params.is_none());
    assert_eq!(
        config.extra,
        BTreeMap::from([(String::from("litellm_params"), Value::Null)])
    );
    assert_eq!(serde_json::to_value(config).unwrap(), input);
}

#[test]
fn file_counts_fields_are_typed() {
    let counts: VectorStoreFileCounts = serde_json::from_str(
        r#"{"in_progress":1,"completed":2,"failed":3,"cancelled":4,"total":10}"#,
    )
    .unwrap();
    assert_eq!(counts.in_progress, Some(1));
    assert_eq!(counts.completed, Some(2));
    assert_eq!(counts.failed, Some(3));
    assert_eq!(counts.cancelled, Some(4));
    assert_eq!(counts.total, Some(10));
    assert!(counts.extra.is_empty());
}
