#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_rates::ModelInfo;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchStatus;
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::gemini_usage::usage::calculate_usage;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_batch_records::create::to_batch_job;
use crate::vertex_batch_records::error::BatchRecordsError;
use crate::vertex_batch_records::model::{
    BatchModelPath, bare_model_name, batch_job_model, decode_vertex_batch_identifier,
    is_unmanaged_gcs_input,
};
use crate::vertex_batch_records::response::{to_batch_list, to_batch_object};
use crate::vertex_batch_records::rows::{
    NativeVertexBatchRowStats, RowCostCalculator, RowCostRequest, VERTEX_AI_PROVIDER,
    embedding_prompt_token_count, is_native_batch_row, native_vertex_batch_row_stats,
    native_vertex_row_usage,
};
use crate::vertex_batch_records::types::{VertexBatchJobPage, VertexBatchPredictionResponse};

const DISPLAY_ID: &str = "00000000-0000-4000-8000-000000000001";
const USAGE_METADATA: &str =
    r#""usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5}"#;
const MODEL_VERSION: &str = r#""gemini-1.5-flash-001""#;

fn utc() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::get("UTC").unwrap(),
    }
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn generate_row(model_version: &str) -> String {
    format!(
        r#"{{"request":{{"contents":[]}},"response":{{"candidates":[{{"content":{{"parts":[{{"text":"hi"}}]}},"finishReason":"STOP"}}],{USAGE_METADATA},"modelVersion":{model_version}}}}}"#
    )
}

fn create_request(input_file_id: &str) -> CreateBatchRequest {
    CreateBatchRequest {
        completion_window: None,
        endpoint: None,
        input_file_id: Some(input_file_id.to_owned()),
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

fn job(source: &str) -> VertexBatchPredictionResponse {
    serde_json::from_str(source).unwrap()
}

fn page(source: &str) -> VertexBatchJobPage {
    serde_json::from_str(source).unwrap()
}

struct FixturePricing {
    failure: bool,
    require_info: bool,
}

impl RowCostCalculator for FixturePricing {
    fn calculate(&self, request: &RowCostRequest<'_>) -> Result<GenericCostResult, TokenCostError> {
        let missing_info = self.require_info && request.model_info.is_none();
        if self.failure || missing_info || request.custom_llm_provider != VERTEX_AI_PROVIDER {
            return Err(TokenCostError::UnknownModel);
        }
        Ok(GenericCostResult {
            prompt_cost: Decimal::new(25, 2),
            completion_cost: Decimal::new(5, 1),
        })
    }
}

fn model_info() -> ModelInfo {
    BTreeMap::from([(
        "input_cost_per_token".to_owned(),
        Scalar::Text("0.000001".to_owned()),
    )])
}

fn stats(row: &Json, model_name: Option<&str>) -> NativeVertexBatchRowStats {
    native_vertex_batch_row_stats(
        row,
        model_name,
        None,
        calculate_usage,
        &FixturePricing {
            failure: false,
            require_info: false,
        },
    )
    .unwrap()
    .unwrap()
}

#[test]
fn a_missing_input_file_id_is_rejected() {
    let error = to_batch_job(
        &CreateBatchRequest::default(),
        None,
        None,
        Uuid::parse_str(DISPLAY_ID).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, BatchRecordsError::InputFileIdRequired));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_create_request_carries_the_file_the_formats_and_the_display_name() {
    let request = create_request("gs://bucket/publishers/google/models/gemini-pro/file.jsonl");
    let job = to_batch_job(
        &request,
        Some("ignored"),
        Some("ignored"),
        Uuid::parse_str(DISPLAY_ID).unwrap(),
    )
    .unwrap();
    assert_eq!(
        job.display_name,
        "litellm-vertex-batch-00000000-0000-4000-8000-000000000001"
    );
    assert_eq!(job.model, "publishers/google/models/gemini-pro");
    assert_eq!(job.input_config.instances_format, "jsonl");
    assert_eq!(
        job.input_config.gcs_source.uris,
        vec!["gs://bucket/publishers/google/models/gemini-pro/file.jsonl".to_owned()]
    );
    assert_eq!(job.output_config.predictions_format, "jsonl");
    assert_eq!(
        job.output_config.gcs_destination.output_uri_prefix,
        "gs://bucket/publishers/google/models/gemini-pro"
    );
}

#[test]
fn an_encoded_publisher_path_wins_over_an_endpoint_before_it() {
    let request = create_request(
        "gs://bucket/endpoints/123/prefix%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-1.5-flash-001%2Ffile.jsonl",
    );
    let job = to_batch_job(&request, Some("proj"), None, Uuid::nil()).unwrap();
    assert_eq!(job.model, "publishers/google/models/gemini-1.5-flash-001");
    assert_eq!(
        job.output_config.gcs_destination.output_uri_prefix,
        "gs://bucket/endpoints/123"
    );
}

#[test]
fn a_publisher_path_without_a_model_falls_through_to_the_endpoint() {
    let model = decode_vertex_batch_identifier(
        b"gs://bucket/publishers/google/models//endpoints/77/file.jsonl",
    )
    .unwrap();
    assert_eq!(
        model,
        BatchModelPath::Endpoint {
            id: "77".to_owned()
        }
    );
}

#[test]
fn the_last_endpoints_segment_names_a_tuned_model() {
    let request = create_request("gs://bucket/endpoints/111/endpoints/222/file.jsonl");
    let job = to_batch_job(&request, Some("proj"), Some("europe-west4"), Uuid::nil()).unwrap();
    assert_eq!(
        job.model,
        "projects/proj/locations/europe-west4/endpoints/222"
    );
}

#[test]
fn a_tuned_endpoint_needs_a_project_and_defaults_its_location() {
    let request = create_request("gs://bucket/endpoints/222/file.jsonl");
    let defaulted = to_batch_job(&request, Some("proj"), None, Uuid::nil()).unwrap();
    assert_eq!(
        defaulted.model,
        "projects/proj/locations/us-central1/endpoints/222"
    );
    let empty_location = create_request("gs://bucket/endpoints/222/file.jsonl");
    let empty_location_job =
        to_batch_job(&empty_location, Some("proj"), Some(""), Uuid::nil()).unwrap();
    assert_eq!(
        empty_location_job.model,
        "projects/proj/locations/us-central1/endpoints/222"
    );
    let missing_project =
        to_batch_job(&request, None, Some("europe-west4"), Uuid::nil()).unwrap_err();
    assert!(matches!(
        missing_project,
        BatchRecordsError::ProjectRequired { .. }
    ));
    assert_eq!(missing_project.status_code(), Some(400));
    let empty_project = create_request("gs://bucket/endpoints/222/file.jsonl");
    let empty_project_error =
        to_batch_job(&empty_project, Some(""), None, Uuid::nil()).unwrap_err();
    assert_eq!(empty_project_error.status_code(), Some(400));
}

#[test]
fn a_gcs_file_without_a_model_path_is_rejected() {
    let missing_model =
        decode_vertex_batch_identifier(b"gs://bucket/plain/file.jsonl").unwrap_err();
    assert!(matches!(
        missing_model,
        BatchRecordsError::ModelMissing { .. }
    ));
    assert_eq!(missing_model.status_code(), Some(400));
    let not_numeric =
        decode_vertex_batch_identifier(b"gs://bucket/endpoints/abc/file.jsonl").unwrap_err();
    assert!(matches!(
        not_numeric,
        BatchRecordsError::ModelMissing { .. }
    ));
    let not_utf8 = decode_vertex_batch_identifier(&[0xff, 0xfe]).unwrap_err();
    assert!(matches!(not_utf8, BatchRecordsError::IdentifierUtf8(_)));
}

#[test]
fn only_a_gs_uri_with_a_model_path_is_an_unmanaged_input() {
    assert!(is_unmanaged_gcs_input(Some(
        "gs://bucket/publishers/google/models/gemini-pro/file.jsonl"
    )));
    assert!(!is_unmanaged_gcs_input(Some(
        "publishers/google/models/gemini-pro/file.jsonl"
    )));
    assert!(!is_unmanaged_gcs_input(Some(
        "gs://bucket/plain/file.jsonl"
    )));
    assert!(!is_unmanaged_gcs_input(Some("")));
    assert!(!is_unmanaged_gcs_input(None));
}

#[test]
fn a_bare_model_name_is_the_last_path_segment() {
    assert_eq!(
        bare_model_name("gs://bucket/publishers/google/models/gemini-pro/file.jsonl").unwrap(),
        "gemini-pro"
    );
    assert_eq!(
        bare_model_name("gs://bucket/endpoints/987/file.jsonl").unwrap(),
        "987"
    );
    bare_model_name("gs://bucket/plain/file.jsonl").unwrap_err();
}

#[test]
fn a_batch_job_model_with_a_project_from_the_identifier_is_a_publisher_path() {
    assert_eq!(
        batch_job_model(
            "gs://bucket/publishers/google/models/gemini-pro/file.jsonl",
            Some("proj"),
            None
        )
        .unwrap(),
        "publishers/google/models/gemini-pro"
    );
}

#[rstest]
#[case::unspecified("JOB_STATE_UNSPECIFIED", BatchStatus::Failed)]
#[case::queued("JOB_STATE_QUEUED", BatchStatus::Validating)]
#[case::pending("JOB_STATE_PENDING", BatchStatus::Validating)]
#[case::running("JOB_STATE_RUNNING", BatchStatus::InProgress)]
#[case::succeeded("JOB_STATE_SUCCEEDED", BatchStatus::Completed)]
#[case::failed("JOB_STATE_FAILED", BatchStatus::Failed)]
#[case::cancelling("JOB_STATE_CANCELLING", BatchStatus::Cancelling)]
#[case::cancelled("JOB_STATE_CANCELLED", BatchStatus::Cancelled)]
#[case::paused("JOB_STATE_PAUSED", BatchStatus::InProgress)]
#[case::expired("JOB_STATE_EXPIRED", BatchStatus::Expired)]
#[case::updating("JOB_STATE_UPDATING", BatchStatus::InProgress)]
#[case::partial("JOB_STATE_PARTIALLY_SUCCEEDED", BatchStatus::Completed)]
fn every_vertex_job_state_maps_to_an_openai_status(
    #[case] state: &str,
    #[case] expected: BatchStatus,
) {
    let response = job(&format!(
        r#"{{"name":"n","createTime":"2024-01-01T00:00:00.000000Z","state":"{state}"}}"#
    ));
    assert_eq!(to_batch_object(&response, &utc()).unwrap().status, expected);
}

#[test]
fn an_unspecified_state_is_the_absent_default() {
    let response = job(r#"{"name":"n","createTime":"2024-01-01T00:00:00.000000Z"}"#);
    assert_eq!(
        to_batch_object(&response, &utc()).unwrap().status,
        BatchStatus::Failed
    );
}

#[rstest]
#[case::unknown(r#"{"state":"JOB_STATE_WHATEVER"}"#)]
#[case::null(r#"{"state":null}"#)]
#[case::wrong_type(r#"{"state":7}"#)]
fn a_state_outside_the_twelve_is_an_error(#[case] extra: &str) {
    let source = format!(r#"{{"name":"n","createTime":"2024-01-01T00:00:00.000000Z",{extra}}}"#);
    let decoded: Result<VertexBatchPredictionResponse, _> = serde_json::from_str(&source);
    let outcome = match decoded {
        Ok(response) => to_batch_object(&response, &utc()).map(|batch| batch.status),
        Err(error) => Err(BatchRecordsError::Decode(error)),
    };
    outcome.unwrap_err();
}

#[test]
fn a_job_response_projects_its_openai_batch_fields() {
    let response = job(r#"{"name":"projects/p/locations/l/batchPredictionJobs/42",
            "createTime":"2024-01-01T00:00:00.000000Z",
            "state":"JOB_STATE_RUNNING",
            "inputConfig":{"instancesFormat":"jsonl","gcsSource":{"uris":["gs://b/in.jsonl","gs://b/other.jsonl"]}},
            "outputInfo":{"gcsOutputDirectory":"gs://b/out//"}}"#);
    let batch = to_batch_object(&response, &utc()).unwrap();
    assert_eq!(batch.id, "42");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(batch.created_at, 1_704_067_200);
    assert_eq!(batch.endpoint, "");
    assert_eq!(batch.input_file_id, "gs://b/in.jsonl");
    assert_eq!(
        batch.output_file_id.as_deref(),
        Some("gs://b/out/predictions.jsonl")
    );
    assert_eq!(batch.error_file_id, None);
}

#[rstest]
#[case::absent(None, None)]
#[case::null(Some("null"), None)]
#[case::empty_object(Some("{}"), None)]
#[case::empty(Some(r#"{"gcsOutputDirectory":""}"#), None)]
#[case::slashes(Some(r#"{"gcsOutputDirectory":"///"}"#), None)]
#[case::directory(
    Some(r#"{"gcsOutputDirectory":"gs://b/out"}"#),
    Some("gs://b/out/predictions.jsonl")
)]
fn an_output_file_appears_only_for_a_nonempty_directory(
    #[case] output_info: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let suffix = output_info.map_or_else(String::new, |info| format!(r#","outputInfo":{info}"#));
    let source = format!(
        r#"{{"name":"n","createTime":"2024-01-01T00:00:00.000000Z","state":"JOB_STATE_SUCCEEDED"{suffix}}}"#
    );
    let batch = to_batch_object(&job(&source), &utc()).unwrap();
    assert_eq!(batch.output_file_id.as_deref(), expected);
}

#[test]
fn a_job_without_a_create_time_is_an_error() {
    let response = job(r#"{"name":"n","state":"JOB_STATE_SUCCEEDED"}"#);
    let error = to_batch_object(&response, &utc()).unwrap_err();
    assert!(matches!(error, BatchRecordsError::CreateTime(_)));
}

#[test]
fn a_job_with_an_empty_input_config_has_an_empty_input_file_id() {
    let response = job(
        r#"{"name":"","createTime":"2024-01-01T00:00:00.000000Z","state":"JOB_STATE_SUCCEEDED",
            "inputConfig":{},"outputInfo":{"gcsOutputDirectory":"gs://b/out"}}"#,
    );
    let batch = to_batch_object(&response, &utc()).unwrap();
    assert_eq!(batch.input_file_id, "");
    assert_eq!(batch.id, "");
}

#[test]
fn a_non_utc_job_create_time_reads_the_injected_zone() {
    let response = job(
        r#"{"name":"n","createTime":"2024-07-01T12:00:00.000000Z","state":"JOB_STATE_SUCCEEDED"}"#,
    );
    let local = LocalTimeContext {
        zone: TimeZone::get("America/New_York").unwrap(),
    };
    assert_eq!(
        to_batch_object(&response, &local).unwrap().created_at,
        1_719_849_600
    );
}

#[test]
fn a_page_preserves_order_ids_and_its_next_token() {
    let page = page(
        r#"{"batchPredictionJobs":[
            {"name":"projects/p/locations/l/batchPredictionJobs/1","createTime":"2024-01-01T00:00:00.000000Z","state":"JOB_STATE_SUCCEEDED"},
            {"name":"projects/p/locations/l/batchPredictionJobs/2","createTime":"2024-01-01T00:00:00.000000Z","state":"JOB_STATE_RUNNING"}],
            "nextPageToken":"next"}"#,
    );
    let list = to_batch_list(&page, &utc()).unwrap();
    assert_eq!(list.first_id.as_deref(), Some("1"));
    assert_eq!(list.last_id.as_deref(), Some("2"));
    assert!(list.has_more);
    assert_eq!(list.next_page_token.as_deref(), Some("next"));
    assert_eq!(list.data.len(), 2);
    assert_eq!(list.data[1].status, BatchStatus::InProgress);
}

#[rstest]
#[case::absent("{}", false, None)]
#[case::null(r#"{"batchPredictionJobs":null}"#, false, None)]
#[case::empty(r#"{"batchPredictionJobs":[]}"#, false, None)]
#[case::empty_token(r#"{"nextPageToken":""}"#, false, Some(""))]
#[case::token(r#"{"nextPageToken":"cursor"}"#, true, Some("cursor"))]
fn an_empty_or_tokenless_page_reports_no_more(
    #[case] source: &str,
    #[case] has_more: bool,
    #[case] token: Option<&str>,
) {
    let list = to_batch_list(&page(source), &utc()).unwrap();
    assert_eq!(list.has_more, has_more);
    assert_eq!(list.next_page_token.as_deref(), token);
    assert_eq!(list.first_id, None);
    assert_eq!(list.last_id, None);
    assert!(list.data.is_empty());
}

#[rstest]
#[case::integer(r#"{"response":{"embedding":{"values":[1]},"tokenCount":12}}"#, 12)]
#[case::digit_string(r#"{"response":{"embedding":{"values":[1]},"tokenCount":"7"}}"#, 7)]
#[case::wrong_string(r#"{"response":{"embedding":{"values":[1]},"tokenCount":"7x"}}"#, 0)]
#[case::signed_string(r#"{"response":{"embedding":{"values":[1]},"tokenCount":"+7"}}"#, 0)]
#[case::float(r#"{"response":{"embedding":{"values":[1]},"tokenCount":7.5}}"#, 0)]
#[case::absent(r#"{"response":{"embedding":{"values":[1]}}}"#, 0)]
#[case::metadata(r#"{"response":{"embedding":{"values":[1]},"usageMetadata":{"promptTokenCount":9,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9}],"totalTokenCount":9}}}"#, 9)]
fn an_embedding_row_without_usage_metadata_bills_its_token_count(
    #[case] source: &str,
    #[case] prompt_tokens: i64,
) {
    let stats = stats(&json(source), None);
    assert_eq!(stats.usage.prompt_tokens, prompt_tokens);
    assert_eq!(stats.usage.completion_tokens, 0);
    assert_eq!(stats.usage.total_tokens, prompt_tokens);
    assert_eq!(stats.total_tokens, prompt_tokens);
    assert_eq!(stats.model, None);
    assert_eq!(stats.prompt_cost, Decimal::ZERO);
}

#[rstest]
#[case::not_an_object(r#"[{"response":{}}]"#)]
#[case::no_response(r#"{"request":{}}"#)]
#[case::string_response(r#"{"response":"text"}"#)]
#[case::not_embedding(r#"{"response":{"tokenCount":4}}"#)]
#[case::empty_embedding(r#"{"response":{"embedding":[]}}"#)]
#[case::null_candidates(
    r#"{"response":{"candidates":null,"usageMetadata":{"promptTokenCount":1}}}"#
)]
#[case::null_response_id(
    r#"{"response":{"responseId":null,"usageMetadata":{"promptTokenCount":1}}}"#
)]
#[case::string_token_count(r#"{"response":{"usageMetadata":{"promptTokenCount":"many"}}}"#)]
#[case::missing_usage(r#"{"response":{"candidates":[],"usageMetadata":null}}"#)]
fn a_row_without_a_usable_response_is_failed(#[case] source: &str) {
    let row = json(source);
    let outcome = native_vertex_batch_row_stats(
        &row,
        Some("deployment"),
        None,
        calculate_usage,
        &FixturePricing {
            failure: false,
            require_info: false,
        },
    )
    .unwrap();
    assert!(outcome.is_none());
}

fn priced_row() -> Json {
    json(&generate_row(MODEL_VERSION))
}

#[test]
fn a_usage_bearing_row_is_priced_from_the_response() {
    let stats = stats(&priced_row(), None);
    assert_eq!(stats.usage.prompt_tokens, 3);
    assert_eq!(stats.usage.completion_tokens, 2);
    assert_eq!(stats.total_tokens, 5);
    assert_eq!(stats.model.as_deref(), Some("gemini-1.5-flash-001"));
    assert_eq!(stats.prompt_cost, Decimal::new(25, 2));
    assert_eq!(stats.completion_cost, Decimal::new(5, 1));
}

#[rstest]
#[case::deployment(Some("my-deployment"), Some("my-deployment"))]
#[case::wildcard_version(Some("vertex_ai/*"), Some("gemini-1.5-flash-001"))]
#[case::no_deployment_version(None, Some("gemini-1.5-flash-001"))]
#[case::empty_deployment(Some(""), Some("gemini-1.5-flash-001"))]
fn the_priced_model_prefers_the_deployment_then_the_version(
    #[case] model_name: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let stats = stats(&priced_row(), model_name);
    assert_eq!(stats.model.as_deref(), expected);
}

#[rstest]
#[case::wildcard(Some("vertex_ai/*"), Some("vertex_ai/*"))]
#[case::empty_deployment(Some(""), Some(""))]
#[case::no_deployment(None, None)]
fn a_non_string_model_version_falls_back_to_the_supplied_model(
    #[case] model_name: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let row = json(&generate_row("7"));
    let stats = stats(&row, model_name);
    assert_eq!(stats.model.as_deref(), expected);
    if expected.is_none() {
        assert_eq!(stats.prompt_cost, Decimal::ZERO);
    } else {
        assert_eq!(stats.prompt_cost, Decimal::new(25, 2));
    }
}

#[test]
fn a_row_without_any_model_keeps_its_usage_and_costs_nothing() {
    let row = json(&generate_row("null"));
    let stats = stats(&row, None);
    assert_eq!(stats.usage.prompt_tokens, 3);
    assert_eq!(stats.total_tokens, 5);
    assert_eq!(stats.model, None);
    assert_eq!(stats.prompt_cost, Decimal::ZERO);
}

#[test]
fn a_pricing_failure_keeps_the_usage_and_returns_zero_cost() {
    let row = priced_row();
    let stats = native_vertex_batch_row_stats(
        &row,
        None,
        None,
        calculate_usage,
        &FixturePricing {
            failure: true,
            require_info: false,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(stats.usage.prompt_tokens, 3);
    assert_eq!(stats.usage.completion_tokens, 2);
    assert_eq!(stats.model.as_deref(), Some("gemini-1.5-flash-001"));
    assert_eq!(stats.prompt_cost, Decimal::ZERO);
    assert_eq!(stats.completion_cost, Decimal::ZERO);
}

#[test]
fn the_row_calculator_receives_the_model_info() {
    let info = model_info();
    let priced = native_vertex_batch_row_stats(
        &priced_row(),
        None,
        Some(&info),
        calculate_usage,
        &FixturePricing {
            failure: false,
            require_info: true,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(priced.prompt_cost, Decimal::new(25, 2));
    let unpriced = native_vertex_batch_row_stats(
        &priced_row(),
        None,
        None,
        calculate_usage,
        &FixturePricing {
            failure: false,
            require_info: true,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(unpriced.prompt_cost, Decimal::ZERO);
}

#[test]
fn a_total_token_count_falls_back_to_the_component_sum() {
    let source =
        generate_row(MODEL_VERSION).replace(r#""totalTokenCount":5"#, r#""totalTokenCount":0"#);
    let stats = stats(&json(&source), None);
    assert_eq!(stats.total_tokens, 5);
    assert_eq!(stats.usage.total_tokens, 0);
}

#[test]
fn native_rows_are_objects_with_a_request() {
    assert!(is_native_batch_row(&json(r#"{"request":{}}"#)));
    assert!(!is_native_batch_row(&json(r#"{"request":"x"}"#)));
    assert!(!is_native_batch_row(&json("[]")));
}

#[test]
fn the_embedding_prompt_count_reads_metadata_before_the_root_fallback() {
    let metadata =
        json(r#"{"embedding":{},"usageMetadata":{"promptTokenCount":4},"tokenCount":9}"#);
    let Json::Object(metadata_fields) = metadata else {
        panic!("object");
    };
    assert_eq!(embedding_prompt_token_count(&metadata_fields), 4);
    let root = json(r#"{"embedding":{},"tokenCount":9}"#);
    let Json::Object(root_fields) = root else {
        panic!("object");
    };
    assert_eq!(embedding_prompt_token_count(&root_fields), 9);
}

#[test]
fn the_row_usage_entry_rejects_a_non_response() {
    let response = IndexMap::new();
    assert!(
        native_vertex_row_usage(&response, &calculate_usage)
            .unwrap()
            .is_none()
    );
}

#[rstest]
fn replays_the_vertex_batch_records_corpus(
    #[files("fuzz/corpus/vertex_batch_records/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_vertex_batch_identifier(&bytes));
    if let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(&bytes)
        && let Some(Json::Object(response)) = fields.get("response")
    {
        drop(native_vertex_row_usage(response, &calculate_usage));
    }
}
