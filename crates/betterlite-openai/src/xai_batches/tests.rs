#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::future::{Future, ready};
use std::mem::take;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use crate::api::batch_records::{BatchRecordsError, CreateBatchRequest, WireDecodeLimits};
use crate::batches::records::{BatchEndpoint, BatchObjectKind, BatchStatus};
use crate::xai_batches::error::XaiBatchesError;
use crate::xai_batches::execution::{CallContext, XaiBatches};
use crate::xai_batches::jsonl::results_to_openai_jsonl;
use crate::xai_batches::records::{
    XaiBatch, XaiBatchList, XaiBatchResult, XaiBatchResultsPage, decode_xai_batch,
    is_xai_batch_results_id,
};
use crate::xai_batches::transformation::{
    DEFAULT_BATCH_ENDPOINT, XAI_API_BASE, XAI_RESULTS_PAGE_SIZE, create_endpoint, get_xai_api_base,
    get_xai_auth_headers, raise_for_xai_status, results_params, to_create_batch_body,
    to_litellm_batch, to_openai_batch_list, xai_batch_status, xai_batches_url,
};
use crate::xai_batches::transport::XaiBatchTransport;

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 1 << 20,
        max_event_bytes: 1 << 10,
        max_items: 1000,
    }
}

fn settings() -> Settings {
    Settings {
        xai_key: None,
        generic_api_key: None,
        environment_key: None,
        api_base: None,
    }
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn batch(source: &str) -> XaiBatch {
    serde_json::from_str(source).unwrap()
}

fn create_request(input_file_id: Option<&str>, name: Option<&str>) -> CreateBatchRequest {
    CreateBatchRequest {
        completion_window: None,
        endpoint: None,
        input_file_id: input_file_id.map(str::to_owned),
        metadata: name.map(|value| IndexMap::from([("name".to_owned(), value.to_owned())])),
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: HeaderMap,
}

struct Recorded {
    method: String,
    url: String,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

#[derive(Default)]
struct Script {
    replies: VecDeque<Reply>,
    calls: Vec<Recorded>,
}

struct Fixture {
    script: Mutex<Script>,
    fail: bool,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(Script {
                replies: VecDeque::from(replies),
                calls: Vec::new(),
            }),
            fail: false,
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(Script::default()),
            fail: true,
        })
    }

    fn only_call(&self) -> Recorded {
        let mut script = self.script.lock().unwrap();
        assert_eq!(script.calls.len(), 1);
        script.calls.remove(0)
    }

    fn calls(&self) -> Vec<Recorded> {
        let mut script = self.script.lock().unwrap();
        take(&mut script.calls)
    }
}

fn request_body_text(body: RequestBody) -> Vec<u8> {
    match body {
        RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
        RequestBody::Json(bytes) => bytes,
        RequestBody::Bytes { data, .. } => data,
    }
}

impl XaiBatchTransport for Arc<Fixture> {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let reply = {
            let mut script = self.script.lock().unwrap();
            let body = request_body_text(request.body);
            script.calls.push(Recorded {
                method: request.method.as_str().to_owned(),
                url: request.url.to_string(),
                query: request.query,
                headers: request.headers,
                body,
                deadline: request.deadline,
            });
            if self.fail {
                None
            } else {
                Some(script.replies.pop_front().expect("a scripted reply"))
            }
        };
        let Some(scripted) = reply else {
            return ready(Err(TransportError::Closed));
        };
        ready(Ok(UpstreamResponse {
            status: scripted.status,
            headers: scripted.headers,
            body: scripted.body,
        }))
    }
}

fn reply(body: &str) -> Reply {
    Reply {
        status: 200,
        body: body.as_bytes().to_vec(),
        headers: HeaderMap::new(),
    }
}

fn api(fixture: &Arc<Fixture>) -> XaiBatches<Arc<Fixture>> {
    XaiBatches {
        transport: Arc::clone(fixture),
        settings: settings(),
        limits: limits(),
    }
}

fn context(api_key: &SecretString, timeout: Duration) -> CallContext<'_> {
    CallContext {
        api_base: None,
        api_key: Some(api_key),
        timeout,
    }
}

fn anonymous(timeout: Duration) -> CallContext<'static> {
    CallContext {
        api_base: None,
        api_key: None,
        timeout,
    }
}

#[rstest]
#[case::default(None, None, "https://api.x.ai")]
#[case::environment(None, Some("https://env.example"), "https://env.example")]
#[case::explicit(
    Some("https://explicit.example"),
    Some("https://env.example"),
    "https://explicit.example"
)]
#[case::trailing_slash(Some("https://explicit.example/"), None, "https://explicit.example")]
#[case::one_v1(Some("https://explicit.example/v1"), None, "https://explicit.example")]
#[case::one_v1_after_slash(Some("https://explicit.example/v1/"), None, "https://explicit.example")]
#[case::two_v1(
    Some("https://explicit.example/v1/v1"),
    None,
    "https://explicit.example/v1"
)]
fn base_resolution_is_explicit_then_environment_then_default(
    #[case] api_base: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = settings();
    settings.api_base = environment.map(str::to_owned);
    assert_eq!(get_xai_api_base(api_base, &settings), expected);
}

#[test]
fn the_default_base_constant_keeps_its_v1() {
    assert_eq!(XAI_API_BASE, "https://api.x.ai/v1");
}

#[test]
fn url_encodes_the_batch_id_as_one_path_segment() {
    let url = xai_batches_url(None, Some("batch_1/a b"), "/results", &settings()).unwrap();
    assert_eq!(url, "https://api.x.ai/v1/batches/batch_1%2Fa%20b/results");
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
fn url_rejects_a_batch_id_that_is_no_path_segment(#[case] batch_id: &str) {
    let error = xai_batches_url(None, Some(batch_id), "", &settings()).unwrap_err();
    assert!(matches!(error, XaiBatchesError::Path { .. }));
}

#[test]
fn url_without_a_batch_id_is_the_batches_collection() {
    assert_eq!(
        xai_batches_url(None, None, "", &settings()).unwrap(),
        "https://api.x.ai/v1/batches"
    );
}

#[test]
fn auth_replaces_a_caller_authorization_in_any_case() {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_static("Bearer old"));
    headers.insert("x-caller", HeaderValue::from_static("kept"));
    let replaced = get_xai_auth_headers(headers, Some(&secret("fresh")), &settings()).unwrap();
    let authorization = replaced.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization.to_str().unwrap(), "Bearer fresh");
    assert_eq!(replaced.get_all(AUTHORIZATION).iter().count(), 1);
    assert_eq!(replaced.get("x-caller").unwrap().to_str().unwrap(), "kept");
}

#[rstest]
#[case::explicit(Some("explicit"), Some("module"), Some("environment"), "explicit")]
#[case::module(None, Some("module"), Some("environment"), "module")]
#[case::environment(None, None, Some("environment"), "environment")]
fn auth_reads_explicit_then_module_then_environment_key(
    #[case] api_key: Option<&str>,
    #[case] xai_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = settings();
    settings.xai_key = xai_key.map(|key| SecretString::from(key.to_owned()));
    settings.environment_key = environment.map(|key| SecretString::from(key.to_owned()));
    let key = api_key.map(secret);
    let headers = get_xai_auth_headers(HeaderMap::new(), key.as_ref(), &settings).unwrap();
    let authorization = headers.get(AUTHORIZATION).unwrap();
    assert_eq!(
        authorization.to_str().unwrap(),
        format!("Bearer {expected}")
    );
}

#[test]
fn auth_without_any_key_is_a_401() {
    let error = get_xai_auth_headers(HeaderMap::new(), None, &settings()).unwrap_err();
    assert_eq!(error.status_code(), Some(401));
    assert!(matches!(error, XaiBatchesError::MissingApiKey));
}

#[rstest]
#[case::batch("batch_1", true)]
#[case::prefix_only("batch_", true)]
#[case::file("file-1", false)]
#[case::empty("", false)]
#[case::inner("my-batch_1", false)]
fn results_file_ids_are_recognized_by_their_batch_prefix(
    #[case] file_id: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_xai_batch_results_id(file_id), expected);
}

#[test]
fn create_body_holds_only_the_name_and_the_input_file() {
    let mut request = create_request(Some("file-1"), Some("team-batch"));
    request.endpoint = Some(BatchEndpoint::Embeddings);
    let body = to_create_batch_body(&request).unwrap();
    assert_eq!(body.name, "team-batch");
    assert_eq!(body.input_file_id, "file-1");
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"name":"team-batch","input_file_id":"file-1"}"#
    );
}

#[rstest]
#[case::no_metadata(None)]
#[case::empty_name(Some(""))]
fn create_body_defaults_the_name(#[case] name: Option<&str>) {
    let request = create_request(Some("file-1"), name);
    assert_eq!(
        to_create_batch_body(&request).unwrap().name,
        "litellm-batch"
    );
}

#[test]
fn create_body_defaults_a_metadata_without_a_name() {
    let mut request = create_request(Some("file-1"), None);
    request.metadata = Some(IndexMap::from([("team".to_owned(), "ml".to_owned())]));
    assert_eq!(
        to_create_batch_body(&request).unwrap().name,
        "litellm-batch"
    );
}

#[test]
fn create_body_takes_a_name_from_the_metadata() {
    let request = create_request(Some("file-1"), Some("team-batch"));
    assert_eq!(to_create_batch_body(&request).unwrap().name, "team-batch");
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn create_body_requires_an_input_file(#[case] input_file_id: Option<&str>) {
    let error = to_create_batch_body(&create_request(input_file_id, None)).unwrap_err();
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(error, XaiBatchesError::InputFileRequired));
}

#[rstest]
#[case::default(None, "/v1/chat/completions")]
#[case::chat(Some(BatchEndpoint::ChatCompletions), "/v1/chat/completions")]
#[case::embeddings(Some(BatchEndpoint::Embeddings), "/v1/embeddings")]
#[case::video_edits(Some(BatchEndpoint::VideoEdits), "/v1/videos/edits")]
fn create_endpoint_defaults_and_keeps_the_caller_value(
    #[case] endpoint: Option<BatchEndpoint>,
    #[case] expected: &str,
) {
    let mut request = create_request(Some("file-1"), None);
    request.endpoint = endpoint;
    assert_eq!(create_endpoint(&request), expected);
}

#[rstest]
#[case::xai_cancel(
    r#"{"batch_id":"b","cancel_time":"2024-01-02","cancel_by_xai_message":"bad input","state":{"num_requests":3,"num_pending":2}}"#,
    BatchStatus::Failed
)]
#[case::caller_cancel(
    r#"{"batch_id":"b","cancel_time":"2024-01-02","state":{"num_requests":3,"num_pending":2}}"#,
    BatchStatus::Cancelled
)]
#[case::zero_requests(
    r#"{"batch_id":"b","state":{"num_requests":0,"num_pending":2}}"#,
    BatchStatus::Validating
)]
#[case::pending(
    r#"{"batch_id":"b","state":{"num_requests":3,"num_pending":2,"num_success":1}}"#,
    BatchStatus::InProgress
)]
#[case::completed(
    r#"{"batch_id":"b","state":{"num_requests":2,"num_pending":0,"num_success":2}}"#,
    BatchStatus::Completed
)]
#[case::absent_state(r#"{"batch_id":"b"}"#, BatchStatus::Validating)]
fn counters_and_cancellation_derive_the_status(
    #[case] source: &str,
    #[case] expected: BatchStatus,
) {
    assert_eq!(xai_batch_status(&batch(source)), expected);
}

#[rstest]
#[case::date_only("2024-01-02", 1_704_153_600)]
#[case::zulu("2024-01-02T03:04:05Z", 1_704_164_645)]
#[case::offset("2024-01-02T03:04:05+02:00", 1_704_157_445)]
#[case::fraction("2024-01-02T03:04:05.987654Z", 1_704_164_645)]
#[case::space_separator("2024-01-02 03:04:05", 1_704_164_645)]
fn creation_times_parse_rfc3339_and_date_only(#[case] create_time: &str, #[case] expected: i64) {
    let source = format!(r#"{{"batch_id":"b","create_time":"{create_time}"}}"#);
    let projected = to_litellm_batch(&batch(&source), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(projected.created_at, expected);
}

#[rstest]
#[case::invalid("not-a-date")]
#[case::empty("")]
fn an_unparsable_creation_time_is_zero(#[case] create_time: &str) {
    let source = format!(r#"{{"batch_id":"b","create_time":"{create_time}"}}"#);
    let projected = to_litellm_batch(&batch(&source), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(projected.created_at, 0);
}

#[test]
fn a_missing_creation_time_is_zero_and_missing_dates_are_null() {
    let projected = to_litellm_batch(&batch(r#"{"batch_id":"b"}"#), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(projected.created_at, 0);
    assert_eq!(projected.expires_at, None);
    assert_eq!(projected.cancelled_at, None);
    assert_eq!(projected.failed_at, None);
}

#[test]
fn an_invalid_expiry_is_null_while_creation_stays_zero() {
    let projected = to_litellm_batch(
        &batch(r#"{"batch_id":"b","create_time":"nope","expire_time":"later"}"#),
        DEFAULT_BATCH_ENDPOINT,
    );
    assert_eq!(projected.created_at, 0);
    assert_eq!(projected.expires_at, None);
}

#[test]
fn a_cancelled_batch_marks_the_cancel_time_as_cancelled() {
    let projected = to_litellm_batch(
        &batch(r#"{"batch_id":"b","cancel_time":"2024-01-02"}"#),
        DEFAULT_BATCH_ENDPOINT,
    );
    assert_eq!(projected.status, BatchStatus::Cancelled);
    assert_eq!(projected.cancelled_at, Some(1_704_153_600));
    assert_eq!(projected.failed_at, None);
}

#[test]
fn an_xai_cancellation_marks_the_batch_failed_with_the_message() {
    let source =
        r#"{"batch_id":"b","cancel_time":"2024-01-02","cancel_by_xai_message":"bad input"}"#;
    let projected = to_litellm_batch(&batch(source), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(projected.status, BatchStatus::Failed);
    assert_eq!(projected.failed_at, Some(1_704_153_600));
    assert_eq!(projected.cancelled_at, None);
    let failures = projected.errors.unwrap();
    assert_eq!(failures.object.as_deref(), Some("list"));
    let failure = &failures.data.unwrap()[0];
    assert_eq!(failure.message.as_deref(), Some("bad input"));
    assert_eq!(failure.code, None);
}

#[test]
fn only_a_completed_batch_publishes_its_own_output_file() {
    let completed = to_litellm_batch(
        &batch(r#"{"batch_id":"batch_out","state":{"num_requests":1,"num_success":1}}"#),
        DEFAULT_BATCH_ENDPOINT,
    );
    assert_eq!(completed.output_file_id.as_deref(), Some("batch_out"));
    let running = to_litellm_batch(
        &batch(r#"{"batch_id":"batch_run","state":{"num_requests":1,"num_pending":1}}"#),
        DEFAULT_BATCH_ENDPOINT,
    );
    assert_eq!(running.output_file_id, None);
}

#[test]
fn request_counts_add_cancellations_to_the_failures() {
    let source = r#"{"batch_id":"b","state":{"num_requests":5,"num_success":2,"num_error":1,"num_cancelled":2}}"#;
    let projected = to_litellm_batch(&batch(source), DEFAULT_BATCH_ENDPOINT);
    let counts = projected.request_counts.unwrap();
    assert_eq!((counts.total, counts.completed, counts.failed), (5, 2, 3));
}

#[rstest]
#[case::with_name("team", Some("team"))]
#[case::no_name("", None)]
fn metadata_carries_a_present_name(#[case] name: &str, #[case] expected: Option<&str>) {
    let source = format!(r#"{{"batch_id":"b","name":"{name}"}}"#);
    let projected = to_litellm_batch(&batch(&source), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(
        projected
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("name"))
            .map(String::as_str),
        expected
    );
}

#[test]
fn an_absent_input_file_becomes_an_empty_string() {
    let projected = to_litellm_batch(&batch(r#"{"batch_id":"b"}"#), DEFAULT_BATCH_ENDPOINT);
    assert_eq!(projected.input_file_id, "");
    assert_eq!(projected.completion_window, "24h");
    assert_eq!(projected.object, BatchObjectKind::Batch);
}

#[test]
fn list_reports_first_and_last_ids_from_the_page_rows() {
    let page: XaiBatchList = serde_json::from_str(
        r#"{"batches":[{"batch_id":"batch_a"},{"batch_id":"batch_b"}],"pagination_token":"next"}"#,
    )
    .unwrap();
    let response = to_openai_batch_list(&page);
    assert_eq!(response.first_id.as_deref(), Some("batch_a"));
    assert_eq!(response.last_id.as_deref(), Some("batch_b"));
    assert!(response.has_more);
    assert_eq!(response.next_page_token.as_deref(), Some("next"));
    assert_eq!(
        serde_json::to_value(&response).unwrap()["object"],
        json!("list")
    );
}

#[test]
fn list_token_truthiness_is_independent_of_the_page_length() {
    let empty_with_token: XaiBatchList =
        serde_json::from_str(r#"{"batches":[],"pagination_token":"next"}"#).unwrap();
    let with_token = to_openai_batch_list(&empty_with_token);
    assert!(with_token.has_more);
    assert_eq!(with_token.first_id, None);
    assert_eq!(with_token.last_id, None);

    let full_without_token: XaiBatchList =
        serde_json::from_str(r#"{"batches":[{"batch_id":"batch_a"}],"pagination_token":null}"#)
            .unwrap();
    let without_token = to_openai_batch_list(&full_without_token);
    assert!(!without_token.has_more);
    assert_eq!(without_token.next_page_token, None);
}

#[test]
fn an_empty_token_is_no_cursor() {
    let page: XaiBatchList =
        serde_json::from_str(r#"{"batches":[],"pagination_token":""}"#).unwrap();
    let response = to_openai_batch_list(&page);
    assert!(!response.has_more);
    assert_eq!(response.next_page_token, None);
}

#[rstest]
#[case::unset(None, None, "limit=1000")]
#[case::zero(Some(0), None, "limit=1000")]
#[case::negative(Some(-1), None, "limit=-1")]
#[case::five(Some(5), None, "limit=5")]
#[case::empty_cursor(Some(2), Some(""), "limit=2&pagination_token=")]
#[case::cursor(Some(2), Some("tok"), "limit=2&pagination_token=tok")]
fn page_params_default_a_zero_limit_and_send_an_explicit_empty_cursor(
    #[case] limit: Option<i64>,
    #[case] after: Option<&str>,
    #[case] expected: &str,
) {
    let pairs: Vec<String> = results_params(after, limit)
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    assert_eq!(pairs.join("&"), expected);
}

#[test]
fn the_page_size_is_1000() {
    assert_eq!(XAI_RESULTS_PAGE_SIZE, 1000);
    assert_eq!(
        results_params(None, None),
        [("limit".to_owned(), "1000".to_owned())]
    );
}

fn result(source: &str) -> XaiBatchResult {
    serde_json::from_str(source).unwrap()
}

#[test]
fn jsonl_unwraps_the_first_response_map_value() {
    let source = r#"{"batch_request_id":"r1","batch_result":{"response":{"chat_get_completion":{"id":"chatcmpl-1","x":1},"other":{"id":"ignored"}}}}"#;
    let output = results_to_openai_jsonl(&[result(source)]).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "{\"id\": \"batch_req_r1\", \"custom_id\": \"r1\", \"response\": {\"status_code\": 200, \"request_id\": \"chatcmpl-1\", \"body\": {\"id\": \"chatcmpl-1\", \"x\": 1}}, \"error\": null}\n"
    );
}

#[test]
fn jsonl_uses_the_stringified_error_code_or_request_failed() {
    let output = results_to_openai_jsonl(&[
        result(
            r#"{"batch_request_id":"r1","batch_result":{"error":{"code":42,"message":"boom"}}}"#,
        ),
        result(
            r#"{"batch_request_id":"r2","batch_result":{"error":{"code":"bad","message":"nope"}}}"#,
        ),
        result(r#"{"batch_request_id":"r3","batch_result":{}}"#),
    ])
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "{\"id\": \"batch_req_r1\", \"custom_id\": \"r1\", \"response\": null, \"error\": {\"code\": \"42\", \"message\": \"boom\"}}"
    );
    assert_eq!(
        lines[1],
        "{\"id\": \"batch_req_r2\", \"custom_id\": \"r2\", \"response\": null, \"error\": {\"code\": \"bad\", \"message\": \"nope\"}}"
    );
    assert_eq!(
        lines[2],
        "{\"id\": \"batch_req_r3\", \"custom_id\": \"r3\", \"response\": null, \"error\": {\"code\": \"request_failed\", \"message\": \"xAI returned no response for this request\"}}"
    );
    assert!(text.ends_with('\n'));
}

#[test]
fn an_error_object_is_ignored_when_the_same_result_has_a_body() {
    let source = r#"{"batch_request_id":"r1","batch_result":{"response":{"responses":{"id":"resp_1"}},"error":{"code":9,"message":"ignored"}}}"#;
    let output = results_to_openai_jsonl(&[result(source)]).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("\"error\": null"));
    assert!(!text.contains("ignored"));
}

#[test]
fn jsonl_preserves_unicode_and_python_separators() {
    let source = "{\"batch_request_id\":\"r1\",\"batch_result\":{\"response\":{\"chat\":{\"id\":\"chatcmpl-\u{e9}\",\"text\":\"h\u{e9}llo \u{4e16}\u{754c}\"}}}}";
    let output = results_to_openai_jsonl(&[result(source)]).unwrap();
    let expected = "{\"id\": \"batch_req_r1\", \"custom_id\": \"r1\", \"response\": {\"status_code\": 200, \"request_id\": \"chatcmpl-\u{e9}\", \"body\": {\"id\": \"chatcmpl-\u{e9}\", \"text\": \"h\u{e9}llo \u{4e16}\u{754c}\"}}, \"error\": null}\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
}

#[test]
fn a_body_without_an_id_has_no_request_id() {
    let source = r#"{"batch_request_id":"r1","batch_result":{"response":{"chat":{"text":"x"}}}}"#;
    let output = results_to_openai_jsonl(&[result(source)]).unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("\"request_id\": null")
    );
}

#[test]
fn an_empty_result_list_is_no_jsonl() {
    assert_eq!(results_to_openai_jsonl(&[]).unwrap(), Vec::<u8>::new());
}

#[test]
fn decode_refuses_bytes_past_the_caller_limit() {
    let limits = WireDecodeLimits {
        max_json_bytes: 20,
        max_event_bytes: 0,
        max_items: 0,
    };
    let at_limit = r#"{"batch_id":"b"}"#;
    assert_eq!(at_limit.len(), 16);
    let decoded: XaiBatch = decode_xai_batch(at_limit.as_bytes(), &limits).unwrap();
    assert_eq!(decoded.batch_id, "b");
    let over_limit = r#"{"batch_id":"batch_1"}"#;
    assert!(over_limit.len() > limits.max_json_bytes);
    assert!(matches!(
        decode_xai_batch::<XaiBatch>(over_limit.as_bytes(), &limits),
        Err(XaiBatchesError::Limit(BatchRecordsError::JsonTooLarge {
            limit: 20
        }))
    ));
}

#[test]
fn decode_ignores_unknown_fields_and_requires_a_batch_id() {
    let decoded: XaiBatch =
        decode_xai_batch(br#"{"batch_id":"b","unknown":{"deep":true}}"#, &limits()).unwrap();
    assert_eq!(decoded.batch_id, "b");
    let error = decode_xai_batch::<XaiBatch>(br#"{"name":"b"}"#, &limits()).unwrap_err();
    assert!(matches!(error, XaiBatchesError::Decode { .. }));
}

#[test]
fn upstream_error_statuses_keep_their_headers() {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let response = UpstreamResponse {
        status: 404,
        headers: reply_headers,
        body: b"no such batch".to_vec(),
    };
    let error = raise_for_xai_status(&response).unwrap_err();
    let XaiBatchesError::Status {
        status,
        message,
        headers,
    } = error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(status, 404);
    assert_eq!(message, "no such batch");
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
}

#[test]
fn a_success_status_passes_the_status_check() {
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: Vec::new(),
    };
    raise_for_xai_status(&response).unwrap();
}

#[tokio::test]
async fn create_posts_the_name_and_input_file_only() {
    let fixture = Fixture::new(vec![reply(
        r#"{"batch_id":"batch_1","input_file_id":"file-1","state":{}}"#,
    )]);
    let mut request = create_request(Some("file-1"), Some("team-batch"));
    request.endpoint = Some(BatchEndpoint::Embeddings);
    let key = secret("key");
    let created = api(&fixture)
        .create_batch(&request, context(&key, Duration::from_secs(600)))
        .await
        .unwrap();
    assert_eq!(created.endpoint, "/v1/embeddings");
    assert_eq!(created.id, "batch_1");
    let call = fixture.only_call();
    assert_eq!(call.method, "POST");
    assert_eq!(call.url, "https://api.x.ai/v1/batches");
    assert_eq!(
        call.headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer key"
    );
    assert_eq!(
        String::from_utf8(call.body).unwrap(),
        r#"{"name":"team-batch","input_file_id":"file-1"}"#
    );
    assert_eq!(call.deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn retrieve_and_cancel_use_get_and_the_colon_cancel_suffix() {
    let fixture = Fixture::new(vec![
        reply(r#"{"batch_id":"batch_1","state":{"num_requests":1,"num_pending":1}}"#),
        reply(r#"{"batch_id":"batch_1","state":{"num_requests":1,"num_success":1}}"#),
    ]);
    let client = api(&fixture);
    let key = secret("key");
    let retrieved = client
        .retrieve_batch("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap();
    assert_eq!(retrieved.status, BatchStatus::InProgress);
    assert_eq!(retrieved.endpoint, "/v1/chat/completions");
    let cancelled = client
        .cancel_batch("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap();
    assert_eq!(cancelled.status, BatchStatus::Completed);
    let calls = fixture.calls();
    assert_eq!(calls[0].method, "GET");
    assert_eq!(calls[0].url, "https://api.x.ai/v1/batches/batch_1");
    assert_eq!(calls[1].method, "POST");
    assert_eq!(calls[1].url, "https://api.x.ai/v1/batches/batch_1:cancel");
}

#[tokio::test]
async fn list_sends_the_cursor_and_limit_params() {
    let fixture = Fixture::new(vec![reply(r#"{"batches":[],"pagination_token":null}"#)]);
    let key = secret("key");
    let listed = api(&fixture)
        .list_batches(
            context(&key, Duration::from_secs(600)),
            Some("tok"),
            Some(0),
        )
        .await
        .unwrap();
    assert!(!listed.has_more);
    let call = fixture.only_call();
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.query,
        [
            ("limit".to_owned(), "1000".to_owned()),
            ("pagination_token".to_owned(), "tok".to_owned())
        ]
    );
    assert_eq!(call.url, "https://api.x.ai/v1/batches");
}

#[tokio::test]
async fn results_follow_pages_until_the_token_is_empty() {
    let fixture = Fixture::new(vec![
        reply(
            r#"{"results":[{"batch_request_id":"a","batch_result":{}}],"pagination_token":"p2"}"#,
        ),
        reply(r#"{"results":[{"batch_request_id":"b","batch_result":{}}],"pagination_token":""}"#),
    ]);
    let key = secret("key");
    let content = api(&fixture)
        .batch_results_content("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap();
    assert_eq!(content.status, 200);
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("application/jsonl")
    );
    let text = String::from_utf8(content.body).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"custom_id\": \"a\""));
    assert!(lines[1].contains("\"custom_id\": \"b\""));
    let calls = fixture.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].url, "https://api.x.ai/v1/batches/batch_1/results");
    assert_eq!(calls[0].query, [("limit".to_owned(), "1000".to_owned())]);
    assert_eq!(
        calls[1].query,
        [
            ("limit".to_owned(), "1000".to_owned()),
            ("pagination_token".to_owned(), "p2".to_owned())
        ]
    );
}

#[tokio::test]
async fn results_stop_when_the_token_arrives_with_no_rows() {
    let fixture = Fixture::new(vec![reply(r#"{"results":[],"pagination_token":"next"}"#)]);
    let key = secret("key");
    let content = api(&fixture)
        .batch_results_content("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap();
    assert!(content.body.is_empty());
    assert_eq!(fixture.calls().len(), 1);
}

#[tokio::test]
async fn results_over_the_byte_limit_fail_with_the_limit_error() {
    let fixture = Fixture::new(vec![reply(
        r#"{"results":[{"batch_request_id":"a","batch_result":{}}]}"#,
    )]);
    let client = XaiBatches {
        transport: Arc::clone(&fixture),
        settings: settings(),
        limits: WireDecodeLimits {
            max_json_bytes: 16,
            max_event_bytes: 0,
            max_items: 0,
        },
    };
    let key = secret("key");
    let error = client
        .batch_results_content("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        XaiBatchesError::Limit(BatchRecordsError::JsonTooLarge { limit: 16 })
    ));
}

#[tokio::test]
async fn an_upstream_status_error_keeps_the_status() {
    let mut reply_headers = HeaderMap::new();
    reply_headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    let fixture = Fixture::new(vec![Reply {
        status: 429,
        body: b"slow down".to_vec(),
        headers: reply_headers,
    }]);
    let key = secret("key");
    let error = api(&fixture)
        .retrieve_batch("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap_err();
    let XaiBatchesError::Status {
        status, headers, ..
    } = error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(status, 429);
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
}

#[tokio::test]
async fn a_transport_failure_is_reported_as_such() {
    let fixture = Fixture::failing();
    let key = secret("key");
    let error = api(&fixture)
        .retrieve_batch("batch_1", context(&key, Duration::from_secs(600)))
        .await
        .unwrap_err();
    assert!(matches!(error, XaiBatchesError::Transport { .. }));
}

#[tokio::test]
async fn a_missing_key_fails_before_the_transport_runs() {
    let fixture = Fixture::new(Vec::new());
    let error = api(&fixture)
        .retrieve_batch("batch_1", anonymous(Duration::from_secs(600)))
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), Some(401));
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn the_call_deadline_is_the_callers_timeout() {
    let fixture = Fixture::new(vec![reply(r#"{"batch_id":"batch_1"}"#)]);
    let key = secret("key");
    api(&fixture)
        .retrieve_batch("batch_1", context(&key, Duration::from_millis(250)))
        .await
        .unwrap();
    assert_eq!(fixture.only_call().deadline, Duration::from_millis(250));
}

fn decode_tagged(bytes: &[u8]) -> Result<(), XaiBatchesError> {
    let Some((tag, body)) = bytes.split_first() else {
        return Ok(());
    };
    match tag {
        1 => decode_xai_batch::<XaiBatchList>(body, &limits()).map(drop),
        2 => decode_xai_batch::<XaiBatchResultsPage>(body, &limits()).map(drop),
        _ => decode_xai_batch::<XaiBatch>(body, &limits()).map(drop),
    }
}

fn replay_corpus() -> usize {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/xai_batch");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in &paths {
        let bytes = fs::read(path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let decoded = decode_tagged(&bytes);
        if name.starts_with("valid-") {
            decoded.unwrap_or_else(|error| panic!("{name}: {error}"));
        } else {
            assert!(decoded.is_err(), "{name}: {decoded:?}");
        }
    }
    paths.len()
}

#[test]
fn replays_xai_batch_corpus() {
    let replayed = replay_corpus();
    assert!(replayed > 0, "no xai batch corpus seeds");
}

#[test]
fn replays_the_xai_batch_corpus() {
    let replayed = replay_corpus();
    assert!(replayed > 0, "no xai batch corpus seeds");
}
