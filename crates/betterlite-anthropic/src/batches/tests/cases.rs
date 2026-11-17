use std::convert::Infallible;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    FixedCounter, Fixture, NOW, Silent, TIMEOUT, ZONE_SECONDS, batches, clock, config, configured,
    has_header, header, header_map, instant, logging_env, logging_state, redactor, secret,
};
use crate::batches::config::{AnthropicBatchesConfig, BatchesConfig, Environment};
use crate::batches::decode::DecodeError;
use crate::batches::error::BatchesError;
use crate::batches::handler::{RetrieveRequest, default_batch_logging};
use crate::batches::result::{BatchResponse, ResultError, project_batch_results};
use crate::batches::timestamps::parse_timestamp;
use crate::batches::{
    BATCHES_BETA_HEADER, COMPLETION_WINDOW, CREATE_NOT_IMPLEMENTED, DEFAULT_INPUT_FILE_ID,
    ENDPOINT, MISSING_API_KEY, MISSING_API_KEY_VALIDATOR, PARSE_PREFIX,
};
use crate::response::Logging;
use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use betterlite_logging::logging_state::LoggingState;

const LIFECYCLE: &str = r#"{"id":"msgbatch_9","processing_status":"ended","created_at":"2024-01-01T00:00:00Z","ended_at":"2024-01-01T01:00:00Z","request_counts":{"succeeded":1}}"#;

#[derive(Clone, Copy)]
enum Scenario {
    BatchCreateRequests,
    BatchStatusCounts,
    BatchEndedOutputFileId,
    BatchErrorCounts,
    BatchRetrieve,
    BatchResultResponse,
    BatchTimestamps,
    BatchAuthHeaders,
}

#[rstest]
#[case::case_1_batch_create_requests(Scenario::BatchCreateRequests)]
#[case::case_2_batch_status_counts(Scenario::BatchStatusCounts)]
#[case::case_3_batch_ended_output_file_id(Scenario::BatchEndedOutputFileId)]
#[case::case_4_batch_error_counts(Scenario::BatchErrorCounts)]
#[case::case_5_batch_retrieve(Scenario::BatchRetrieve)]
#[case::case_6_batch_result_response(Scenario::BatchResultResponse)]
#[case::case_7_batch_timestamps(Scenario::BatchTimestamps)]
#[case::case_8_batch_auth_headers(Scenario::BatchAuthHeaders)]
#[tokio::test]
async fn batches_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::BatchCreateRequests => batch_create_requests(),
        Scenario::BatchStatusCounts => batch_status_counts(),
        Scenario::BatchEndedOutputFileId => batch_ended_output_file_id(),
        Scenario::BatchErrorCounts => batch_error_counts(),
        Scenario::BatchRetrieve => batch_retrieve().await,
        Scenario::BatchResultResponse => batch_result_response(),
        Scenario::BatchTimestamps => batch_timestamps(),
        Scenario::BatchAuthHeaders => batch_auth_headers(),
    }
}

fn transform(config: &AnthropicBatchesConfig, body: &str) -> BatchObject {
    config
        .transform_retrieve_batch_response(body.as_bytes(), &clock())
        .unwrap()
}

fn batch_create_requests() {
    let batched = config(Some("sk-batches"), Some("https://env.example/"));
    assert_eq!(
        batched.get_complete_batch_url(None),
        "https://env.example/v1/messages/batches"
    );
    assert_eq!(
        batched.get_complete_batch_url(Some("")),
        "https://env.example/v1/messages/batches"
    );
    assert_eq!(
        batched.get_complete_batch_url(Some("https://x.example/")),
        "https://x.example/v1/messages/batches"
    );
    assert_eq!(
        batched.get_complete_batch_url(Some("https://x.example/v1/messages/batches")),
        "https://x.example/v1/messages/batches"
    );
    assert_eq!(
        batched.get_complete_batch_url(Some("https://x.example/v1/messages/batches/")),
        "https://x.example/v1/messages/batches/v1/messages/batches"
    );
    let request = batched.transform_create_batch_request().unwrap_err();
    assert!(matches!(request, BatchesError::NotImplemented));
    assert_eq!(request.to_string(), CREATE_NOT_IMPLEMENTED);
    let response = batched.transform_create_batch_response().unwrap_err();
    assert!(matches!(response, BatchesError::NotImplemented));
    assert_eq!(response.to_string(), CREATE_NOT_IMPLEMENTED);
}

fn batch_status_counts() {
    status_defaults();
    status_mapping();
    count_rules();
}

fn status_defaults() {
    let batched = config(Some("sk"), None);
    let running = transform(&batched, r#"{"id":"b1"}"#);
    assert_eq!(running.id, "b1");
    assert_eq!(running.status, BatchStatus::InProgress);
    assert_eq!(running.created_at, NOW);
    assert_eq!(running.in_progress_at, None);
    assert_eq!(running.request_counts.map(|counts| counts.total), Some(0));
    let empty = transform(&batched, "{}");
    assert_eq!(empty.id, "");
    assert_eq!(empty.status, BatchStatus::InProgress);
    let null_status = transform(
        &batched,
        r#"{"processing_status":null,"created_at":"1970-01-01T00:00:00Z"}"#,
    );
    assert_eq!(null_status.status, BatchStatus::InProgress);
    assert_eq!(null_status.in_progress_at, None);
    assert_eq!(null_status.created_at, NOW);
}

fn status_mapping() {
    let batched = config(Some("sk"), None);
    let cases = [
        ("in_progress", BatchStatus::InProgress),
        ("canceling", BatchStatus::Cancelling),
        ("ended", BatchStatus::Completed),
        ("archived", BatchStatus::InProgress),
        ("", BatchStatus::InProgress),
    ];
    for (raw, expected) in cases {
        let body = format!(r#"{{"processing_status":"{raw}"}}"#);
        assert_eq!(transform(&batched, &body).status, expected, "{raw}");
    }
    let cancelling = transform(&batched, r#"{"processing_status":"canceling"}"#);
    assert_eq!(cancelling.output_file_id.as_deref(), Some(""));
}

fn count_rules() {
    let batched = config(Some("sk"), None);
    let counted = transform(
        &batched,
        r#"{"request_counts":{"processing":1,"succeeded":2,"errored":3,"canceled":4,"expired":5}}"#,
    );
    let counts = counted.request_counts.unwrap();
    assert_eq!((counts.total, counts.completed, counts.failed), (15, 2, 3));
    let partial = transform(
        &batched,
        r#"{"request_counts":{"succeeded":2,"errored":3}}"#,
    );
    let counts = partial.request_counts.unwrap();
    assert_eq!((counts.total, counts.completed, counts.failed), (5, 2, 3));
}

fn batch_ended_output_file_id() {
    let batched = config(Some("sk"), None);
    let body = r#"{"id":"msgbatch_1","processing_status":"ended","custom_id":"c1","results_url":"https://results.example","ended_at":"2024-01-01T01:02:03Z","request_counts":{"processing":1,"succeeded":2,"errored":3,"canceled":4,"expired":5}}"#;
    let batch = transform(&batched, body);
    assert_eq!(batch.id, "msgbatch_1");
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.object, BatchObjectKind::Batch);
    assert_eq!(batch.endpoint, ENDPOINT);
    assert_eq!(batch.input_file_id, DEFAULT_INPUT_FILE_ID);
    assert_eq!(batch.completion_window, COMPLETION_WINDOW);
    assert_eq!(batch.output_file_id.as_deref(), Some("msgbatch_1"));
    assert!(batch.errors.is_none());
    assert_eq!(batch.error_file_id, None);
    assert_eq!(batch.finalizing_at, None);
    assert_eq!(batch.failed_at, None);
    assert_eq!(batch.metadata, Some(IndexMap::new()));
    assert_eq!(batch.provider_fields.len(), 0);
    let counts = batch.request_counts.unwrap();
    assert_eq!((counts.total, counts.completed, counts.failed), (15, 2, 3));
    output_file_for_every_status();
}

fn output_file_for_every_status() {
    let batched = config(Some("sk"), None);
    for status in ["in_progress", "canceling", "ended", "other"] {
        let body = format!(r#"{{"id":"msgbatch_{status}","processing_status":"{status}"}}"#);
        let batch = transform(&batched, &body);
        let expected = format!("msgbatch_{status}");
        assert_eq!(batch.output_file_id.as_deref(), Some(expected.as_str()));
        assert_eq!(batch.input_file_id, DEFAULT_INPUT_FILE_ID);
        assert_eq!(batch.metadata, Some(IndexMap::new()));
    }
}

fn batch_error_counts() {
    malformed_and_structural();
    count_overflow();
    provider_errors();
}

fn malformed_and_structural() {
    let batched = config(Some("sk"), None);
    let malformed = batched
        .transform_retrieve_batch_response(b"{not json", &clock())
        .unwrap_err();
    assert!(matches!(malformed, DecodeError::Parse { .. }));
    assert!(malformed.to_string().starts_with(PARSE_PREFIX));
    for body in [
        "[]",
        r#"{"request_counts":null}"#,
        r#"{"request_counts":{"succeeded":"x"}}"#,
        r#"{"request_counts":{"processing":1.5}}"#,
        r#"{"id":5}"#,
        r#"{"processing_status":7}"#,
    ] {
        let error = batched
            .transform_retrieve_batch_response(body.as_bytes(), &clock())
            .unwrap_err();
        assert!(
            matches!(error, DecodeError::Shape { .. }),
            "{body}: {error}"
        );
    }
}

fn count_overflow() {
    let batched = config(Some("sk"), None);
    let error = batched
        .transform_retrieve_batch_response(
            r#"{"request_counts":{"processing":9223372036854775807,"succeeded":9223372036854775807}}"#
                .as_bytes(),
            &clock(),
        )
        .unwrap_err();
    assert!(matches!(error, DecodeError::CountOverflow));
}

fn provider_errors() {
    let headers = header_map(&[("x-request-id", "req-1")]);
    let error = BatchesError::provider("boom", 429, Some(&headers));
    let BatchesError::Provider(provider) = &error else {
        panic!("a status error is a provider error");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "boom");
    assert_eq!(
        provider.headers,
        Some(vec![("x-request-id".to_owned(), "req-1".to_owned())])
    );
    let BatchesError::Provider(unheaded) = BatchesError::provider("boom", 500, None) else {
        panic!("a status error is a provider error");
    };
    assert_eq!(unheaded.status_code, 500);
    assert_eq!(unheaded.headers, None);
}

async fn batch_retrieve() {
    basic_retrieve().await;
    fallback_retrieve().await;
    missing_key().await;
    status_failure().await;
    transport_failure().await;
    invalid_batch_ids().await;
    assert!(
        config(Some("sk"), None)
            .transform_retrieve_batch_request()
            .is_empty()
    );
}

fn retrieve<'call>(
    batch_id: &'call str,
    api_base: Option<&'call str>,
    api_key: Option<&'call SecretString>,
) -> RetrieveRequest<'call> {
    RetrieveRequest {
        batch_id,
        api_base,
        api_key,
        timeout: Some(Duration::from_secs(1)),
        max_retries: Some(3),
    }
}

async fn basic_retrieve() {
    let fixture = Fixture::answering(200, LIFECYCLE.as_bytes());
    let handler = batches(
        &fixture,
        config(Some("sk-batches"), Some("https://custom.example/")),
    );
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("msgbatch_9", &redactor);
    let env = logging_env(&redactor, &silent);
    let request = retrieve("msgbatch_9", Some("https://explicit.example"), None);
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    let batch = handler.retrieve_batch(&request, logging).await.unwrap();
    assert_eq!(batch.id, "msgbatch_9");
    assert_eq!(batch.status, BatchStatus::Completed);
    let sent = fixture.only_request();
    assert_eq!(sent.method, Method::GET);
    assert_eq!(
        sent.url.as_str(),
        "https://explicit.example/v1/messages/batches/msgbatch_9"
    );
    assert_eq!(sent.deadline, TIMEOUT);
    assert_eq!(header(&sent.headers, "x-api-key"), "sk-batches");
    assert_eq!(header(&sent.headers, "anthropic-beta"), BATCHES_BETA_HEADER);
    assert_eq!(header(&sent.headers, "anthropic-version"), "2023-06-01");
    assert_eq!(header(&sent.headers, "accept"), "application/json");
    assert_eq!(header(&sent.headers, "content-type"), "application/json");
    assert_eq!(fixture.sent(), 1);
    assert_logged_call(&state, "msgbatch_9");
}

fn assert_logged_call(state: &LoggingState, batch_id: &str) {
    assert_eq!(state.model, "anthropic/unknown");
    assert_eq!(state.call_type, "batch_retrieve");
    assert_eq!(state.litellm_call_id, format!("batch_retrieve_{batch_id}"));
    assert_eq!(state.function_id, "batch_retrieve");
    assert_eq!(state.stream, Some(false));
    assert_eq!(state.messages, Some(Vec::new()));
    assert_eq!(state.start_time, instant());
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Text(batch_id.to_owned()))
    );
    let Some(Json::Object(details)) = state.model_call_details.get("additional_args") else {
        panic!("pre_call records the additional args");
    };
    assert_eq!(
        details.get("api_base"),
        Some(&Json::Text(
            "https://explicit.example/v1/messages/batches/msgbatch_9".to_owned()
        ))
    );
    assert_eq!(
        details.get("complete_input_dict"),
        Some(&Json::Object(IndexMap::new()))
    );
    let Some(Json::Object(masked)) = details.get("headers") else {
        panic!("pre_call records the header names");
    };
    assert!(masked.contains_key("x-api-key"));
    assert_ne!(
        masked.get("x-api-key"),
        Some(&Json::Text("sk-batches".to_owned()))
    );
    assert_eq!(
        masked.get("accept"),
        Some(&Json::Text("application/json".to_owned()))
    );
}

async fn fallback_retrieve() {
    let fixture = Fixture::answering(200, LIFECYCLE.as_bytes());
    let batched = AnthropicBatchesConfig {
        settings: configured(Some("sk-env"), None, Some("https://env-base.example/")),
    };
    let handler = batches(&fixture, batched);
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("b", &redactor);
    let env = logging_env(&redactor, &silent);
    let empty = secret("");
    let request = retrieve("a b/c?d", Some(""), Some(&empty));
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    handler.retrieve_batch(&request, logging).await.unwrap();
    let sent = fixture.only_request();
    assert_eq!(
        sent.url.as_str(),
        "https://env-base.example/v1/messages/batches/a%20b%2Fc%3Fd"
    );
    assert_eq!(header(&sent.headers, "x-api-key"), "sk-env");
}

async fn missing_key() {
    let fixture = Fixture::answering(200, LIFECYCLE.as_bytes());
    let batched = AnthropicBatchesConfig {
        settings: configured(None, Some("auth-token-only"), None),
    };
    let handler = batches(&fixture, batched);
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("b", &redactor);
    let env = logging_env(&redactor, &silent);
    let request = retrieve("b", None, None);
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    let error = handler.retrieve_batch(&request, logging).await.unwrap_err();
    assert!(matches!(error, BatchesError::MissingApiKey));
    assert_eq!(error.to_string(), MISSING_API_KEY);
    assert_eq!(fixture.sent(), 0);
}

async fn status_failure() {
    let denied = Fixture::answering_with(429, &[("x-request-id", "req-1")], b"slow down");
    let handler = batches(&denied, config(Some("sk"), None));
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("b", &redactor);
    let env = logging_env(&redactor, &silent);
    let request = retrieve("b", None, None);
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    let error = handler.retrieve_batch(&request, logging).await.unwrap_err();
    let BatchesError::Provider(provider) = &error else {
        panic!("a non-2xx reply is a provider error: {error:?}");
    };
    assert_eq!(provider.status_code, 429);
    assert_eq!(provider.message, "slow down");
    assert_eq!(
        provider.headers,
        Some(vec![("x-request-id".to_owned(), "req-1".to_owned())])
    );
}

async fn transport_failure() {
    let failing = Fixture::failing();
    let handler = batches(&failing, config(Some("sk"), None));
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("b", &redactor);
    let env = logging_env(&redactor, &silent);
    let request = retrieve("b", None, None);
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    let error = handler.retrieve_batch(&request, logging).await.unwrap_err();
    assert!(matches!(error, BatchesError::Transport { .. }));
}

async fn invalid_batch_ids() {
    let fixture = Fixture::answering(200, LIFECYCLE.as_bytes());
    let handler = batches(&fixture, config(Some("sk"), None));
    let redactor = redactor();
    let silent = Silent;
    let env = logging_env(&redactor, &silent);
    for batch_id in ["", "."] {
        let mut state = logging_state("b", &redactor);
        let request = retrieve(batch_id, None, None);
        let logging = Logging {
            state: &mut state,
            env: &env,
        };
        let error = handler.retrieve_batch(&request, logging).await.unwrap_err();
        assert!(matches!(error, BatchesError::Segment(_)), "{batch_id}");
    }
    assert_eq!(fixture.sent(), 0);
    let error = config(Some("sk"), None)
        .get_retrieve_batch_url(None, "")
        .unwrap_err();
    assert!(matches!(
        error,
        BatchesError::Segment(HttpError::Required { .. })
    ));
}

fn row(custom_id: &str, text: &str, model: &str, usage: &str) -> String {
    format!(
        r#"{{"custom_id":"{custom_id}","result":{{"type":"succeeded","message":{{"id":"msg_1","type":"message","role":"assistant","model":"{model}","content":[{{"type":"text","text":"{text}"}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{usage}}}}}}}"#
    )
}

fn project(body: &[u8], status: u16) -> (ResponseEnvelope, Result<(), ResultError<Infallible>>) {
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("batch", &redactor);
    let env = logging_env(&redactor, &silent);
    let mut envelope = ResponseEnvelope::new("batch".to_owned());
    let headers = HeaderMap::new();
    let response = BatchResponse {
        status,
        headers: &headers,
        body,
        now: NOW,
    };
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    let result = project_batch_results(&mut envelope, response, logging, &FixedCounter);
    (envelope, result)
}

fn batch_result_response() {
    last_row_wins();
    no_valid_rows();
    aborts_on_bad_rows();
}

fn last_row_wins() {
    let body = [
        row(
            "c1",
            "first",
            "claude-a",
            r#"{"input_tokens":10,"output_tokens":2,"cache_creation_input_tokens":4,"cache_read_input_tokens":6,"output_tokens_details":{"thinking_tokens":1}}"#,
        ),
        String::new(),
        "{not json".to_owned(),
        row(
            "c2",
            "second",
            "claude-b",
            r#"{"input_tokens":3,"output_tokens":5,"output_tokens_details":{"thinking_tokens":2}}"#,
        ),
    ]
    .join("\n");
    let (envelope, result) = project(body.as_bytes(), 200);
    result.unwrap();
    assert_eq!(envelope.model.as_deref(), Some("claude-b"));
    assert_eq!(envelope.created, NOW);
    let choice = envelope.choices.first().unwrap();
    assert_eq!(choice.message.content.as_deref(), Some("second"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    let usage = envelope.usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (23, 7, 30)
    );
    assert_eq!(usage.prompt_tokens_details.text, 13);
    assert_eq!(usage.prompt_tokens_details.cached, 6);
    assert_eq!(usage.prompt_tokens_details.cache_creation, 4);
    assert_eq!(usage.prompt_tokens_details.cache_write, 4);
    assert_eq!(usage.prompt_tokens_details.audio, Some(0));
    assert_eq!(usage.completion_tokens_details.reasoning, Some(3));
    assert_eq!(usage.completion_tokens_details.text, Some(4));
    assert_eq!(usage.completion_tokens_details.accepted_prediction, Some(0));
    assert_eq!(usage.cache_read_input_tokens, 0);
}

fn no_valid_rows() {
    let (envelope, result) = project(b"\n{not json\n   \n", 200);
    result.unwrap();
    let choice = envelope.choices.first().unwrap();
    assert_eq!(choice.message.content, None);
    assert_eq!(envelope.model, None);
    assert_eq!(envelope.created, NOW);
    let usage = envelope.usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (0, 0, 0)
    );
    assert_eq!(usage.prompt_tokens_details.text, 0);
    assert_eq!(usage.prompt_tokens_details.cached, 0);
    assert_eq!(usage.completion_tokens_details.reasoning, Some(0));
}

fn aborts_on_bad_rows() {
    let (_, missing) = project(b"{\"id\":\"x\"}\n", 200);
    assert!(matches!(
        missing,
        Err(ResultError::Decode(DecodeError::Shape { .. }))
    ));
    let (_, not_object) = project(b"\"row\"\n", 200);
    assert!(matches!(
        not_object,
        Err(ResultError::Decode(DecodeError::Shape { .. }))
    ));
    let bad_content = r#"{"custom_id":"c","result":{"message":{"id":"m","type":"message","role":"assistant","model":"m","content":"text","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}}}"#;
    let (_, content) = project(bad_content.as_bytes(), 200);
    assert!(matches!(
        content,
        Err(ResultError::Projection(ResponseError::Decode(_)))
    ));
    let error_row = r#"{"custom_id":"c","result":{"message":{"type":"error","error":{"type":"overloaded_error","message":"busy"}}}}"#;
    let (_, failed) = project(error_row.as_bytes(), 503);
    let Err(ResultError::Projection(ResponseError::Anthropic { status, .. })) = failed else {
        panic!("an error row raises the Anthropic error");
    };
    assert_eq!(status, 503);
}

fn timestamp(value: Json, zone_seconds: i32) -> Option<i64> {
    let zone = chrono::FixedOffset::east_opt(zone_seconds).unwrap();
    parse_timestamp(&value, zone)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn batch_timestamps() {
    utc_forms();
    local_forms();
    rejected_forms();
    status_timestamps();
}

fn utc_forms() {
    for form in [
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:00.123456Z",
        "2024-01-01T00:00:00,5Z",
        "2024-01-01T00:00:00.1234567Z",
        "2024-01-01T00:00Z",
    ] {
        assert_eq!(
            timestamp(text(form), ZONE_SECONDS),
            Some(1_704_067_200),
            "{form}"
        );
    }
    assert_eq!(
        timestamp(text("1969-12-31T23:59:59.5Z"), ZONE_SECONDS),
        Some(0)
    );
    assert_eq!(
        timestamp(text("1970-01-01T00:00:00.5Z"), ZONE_SECONDS),
        Some(0)
    );
    assert_eq!(
        timestamp(text("1969-12-31T23:59:59Z"), ZONE_SECONDS),
        Some(-1)
    );
    assert_eq!(
        timestamp(text("1969-12-31T23:59:59.0Z"), ZONE_SECONDS),
        Some(-1)
    );
}

fn local_forms() {
    for form in [
        "2024-01-01T00:00:00+02:00",
        "2024-01-01T00:00:00+0200",
        "2024-01-01T00:00:00+02",
        "2024-01-01T00:00:00",
        "2024-01-01",
        "2024-01-01 00:00:00",
        "20240101",
    ] {
        assert_eq!(
            timestamp(text(form), ZONE_SECONDS),
            Some(1_704_060_000),
            "{form}"
        );
    }
    assert_eq!(
        timestamp(text("2024-01-01T00:00:00+23:00"), ZONE_SECONDS),
        Some(1_703_984_400)
    );
    assert_eq!(
        timestamp(text("2024-01-01T00:00:00+00:59"), ZONE_SECONDS),
        Some(1_704_063_660)
    );
    assert_eq!(
        timestamp(text("2024-01-01T00:00:00-02:00"), ZONE_SECONDS),
        Some(1_704_074_400)
    );
    assert_eq!(
        timestamp(text("2024-01-01T00:00:00"), -18_000),
        Some(1_704_085_200)
    );
}

fn rejected_forms() {
    for form in [
        "",
        "not-a-date",
        "2024-02-30T00:00:00Z",
        "2024-01-01T24:00:00Z",
        "2024-01-01T00:00:00+24:00",
        "2024-01-01T00:00:00+00:60",
        "2024-01-01T00:00:00.",
        "2024-01-01T",
        "2024-01-011T00:00:00Z",
        "202401011T00:00:00Z",
        "2024/01-01T00:00:00Z",
        "2024-01011T00:00:00Z",
        "2024-01-01T0:00:00Z",
    ] {
        assert_eq!(timestamp(text(form), ZONE_SECONDS), None, "{form}");
    }
    assert_eq!(timestamp(Json::Null, ZONE_SECONDS), None);
    assert_eq!(
        timestamp(Json::Number(serde_json::Number::from(0_u8)), ZONE_SECONDS),
        None
    );
}

fn status_timestamps() {
    let batched = config(Some("sk"), None);
    let epoch = transform(
        &batched,
        r#"{"id":"b","processing_status":"in_progress","created_at":"1970-01-01T00:00:00Z"}"#,
    );
    assert_eq!(epoch.created_at, NOW);
    assert_eq!(epoch.in_progress_at, Some(0));
    let ended = transform(
        &batched,
        r#"{"id":"b","processing_status":"ended","ended_at":"2024-01-01T00:00:00Z","archived_at":"2024-01-04T00:00:00Z","expires_at":"2024-01-05T00:00:00Z"}"#,
    );
    assert_eq!(ended.status, BatchStatus::Completed);
    assert_eq!(ended.completed_at, Some(1_704_067_200));
    assert_eq!(ended.expired_at, Some(1_704_326_400));
    assert_eq!(ended.expires_at, Some(1_704_412_800));
    let cancelling = transform(
        &batched,
        r#"{"id":"b","processing_status":"canceling","cancel_initiated_at":"2024-01-02T00:00:00Z","ended_at":"2024-01-03T00:00:00Z"}"#,
    );
    assert_eq!(cancelling.cancelling_at, Some(1_704_153_600));
    assert_eq!(cancelling.cancelled_at, Some(1_704_240_000));
    let archived = transform(
        &batched,
        r#"{"id":"b","processing_status":"in_progress","archived_at":"1970-01-01T00:00:00Z"}"#,
    );
    assert_eq!(archived.expired_at, None);
    let numeric = transform(&batched, r#"{"id":"b","created_at":12345}"#);
    assert_eq!(numeric.created_at, NOW);
    assert_eq!(numeric.in_progress_at, None);
}

fn batch_auth_headers() {
    explicit_auth();
    bearer_auth();
    missing_auth();
}

fn explicit_auth() {
    let argument = secret("sk-argument");
    let batched = config(Some("sk-explicit"), None);
    let mut headers = header_map(&[
        ("anthropic-beta", ""),
        ("accept", "text/plain"),
        ("content-type", "text/csv"),
        ("x-keep", "keep"),
    ]);
    batched
        .validate_environment(
            &mut headers,
            Environment {
                api_key: Some(&argument),
                api_base: Some("https://custom.example"),
                params_api_base: None,
            },
        )
        .unwrap();
    assert_eq!(header(&headers, "anthropic-beta"), "");
    assert_eq!(header(&headers, "accept"), "application/json");
    assert_eq!(header(&headers, "content-type"), "application/json");
    assert_eq!(header(&headers, "anthropic-version"), "2023-06-01");
    assert_eq!(header(&headers, "x-api-key"), "sk-argument");
    assert_eq!(header(&headers, "x-keep"), "keep");
    assert!(headers.get("x-api-key").unwrap().is_sensitive());
    let mut caller_beta = header_map(&[("anthropic-beta", "caller-beta")]);
    batched
        .validate_environment(
            &mut caller_beta,
            Environment {
                api_key: None,
                api_base: None,
                params_api_base: Some("https://params.example"),
            },
        )
        .unwrap();
    assert_eq!(header(&caller_beta, "anthropic-beta"), "caller-beta");
    assert_eq!(header(&caller_beta, "x-api-key"), "sk-explicit");
}

fn bearer_auth() {
    let token_only = AnthropicBatchesConfig {
        settings: configured(None, Some("auth-token"), None),
    };
    let mut token_headers = HeaderMap::new();
    token_only
        .validate_environment(
            &mut token_headers,
            Environment {
                api_key: None,
                api_base: None,
                params_api_base: None,
            },
        )
        .unwrap();
    assert_eq!(header(&token_headers, "authorization"), "Bearer auth-token");
    assert!(!has_header(&token_headers, "x-api-key"));
    assert_eq!(
        header(&token_headers, "anthropic-beta"),
        BATCHES_BETA_HEADER
    );
    let oauth = AnthropicBatchesConfig {
        settings: configured(Some("sk-ant-oat-abc"), None, None),
    };
    let mut oauth_headers = HeaderMap::new();
    oauth
        .validate_environment(
            &mut oauth_headers,
            Environment {
                api_key: None,
                api_base: None,
                params_api_base: None,
            },
        )
        .unwrap();
    assert_eq!(
        header(&oauth_headers, "authorization"),
        "Bearer sk-ant-oat-abc"
    );
    assert!(!has_header(&oauth_headers, "x-api-key"));
}

fn missing_auth() {
    let none = AnthropicBatchesConfig {
        settings: configured(None, None, None),
    };
    let error = none
        .validate_environment(
            &mut HeaderMap::new(),
            Environment {
                api_key: None,
                api_base: None,
                params_api_base: None,
            },
        )
        .unwrap_err();
    assert!(matches!(error, BatchesError::MissingEnvironmentKey));
    assert_eq!(error.to_string(), MISSING_API_KEY_VALIDATOR);
    assert_eq!(default_batch_logging("x").function_id, "batch_retrieve");
}
