#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::reply::VideoObjectType;
use http::HeaderMap;
use http::header::HeaderValue;
use rstest::rstest;

use super::artifact::ArtifactTransport;
use super::cancel::cancel_response;
use super::content::content_response;
use super::error::VideoLifecycleError;
use super::retrieve::retrieve_response;
use super::unsupported::{UnsupportedOperation, unsupported_request};
use super::{LifecycleOperation, lifecycle_request};
use crate::api::api_boundaries::ApiLimits;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 1024,
    max_event_bytes: 1024,
    max_items: 16,
};
const DEADLINE: Duration = Duration::from_secs(30);
const MISSING_URL: &str = "Video URL not found in response. Video may not be ready yet.";

struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: HeaderMap,
}

#[derive(Clone, Debug)]
struct Sent {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    max_bytes: usize,
}

struct Script {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<Sent>>,
}

impl Script {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn once(status: u16, body: &'static str) -> Self {
        Self::new(vec![Reply {
            status,
            body: body.as_bytes().to_vec(),
            headers: HeaderMap::new(),
        }])
    }

    fn none() -> Self {
        Self::new(Vec::new())
    }

    fn requests(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl ArtifactTransport for Script {
    fn fetch(
        &self,
        request: OutboundRequest,
        max_bytes: usize,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap().to_owned()))
            .collect();
        self.sent.lock().unwrap().push(Sent {
            method: request.method.as_str().to_owned(),
            url: request.url.to_string(),
            headers,
            max_bytes,
        });
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: reply.headers,
            body: reply.body,
        }))
    }
}

#[test]
fn encoded_task_path_decodes_the_wrapper_and_escapes_the_original_id() {
    let wrapped = encode_video_id_with_provider("task/one two", "runwayml", Some("gen4_turbo"));
    let content = lifecycle_request(
        LifecycleOperation::Content {
            variant: Some("thumbnail"),
        },
        &wrapped,
        Some("https://api.test/v1/"),
    )
    .unwrap();
    assert_eq!(content.url, "https://api.test/v1/tasks/task%2Fone%20two");
    assert!(content.data.is_empty());
    let status = lifecycle_request(
        LifecycleOperation::Status,
        &wrapped,
        Some("https://api.test/v1"),
    )
    .unwrap();
    assert_eq!(status.url, "https://api.test/v1/tasks/task%2Fone%20two");
    assert!(status.data.is_empty());
    let cancel = lifecycle_request(
        LifecycleOperation::Cancel,
        &wrapped,
        Some("https://api.test/v1"),
    )
    .unwrap();
    assert_eq!(
        cancel.url,
        "https://api.test/v1/tasks/task%2Fone%20two/cancel"
    );
    assert!(cancel.data.is_empty());
    assert_eq!(
        lifecycle_request(LifecycleOperation::Status, "task_1", None)
            .unwrap()
            .url,
        "https://api.dev.runwayml.com/v1/tasks/task_1"
    );
}

#[test]
fn content_request_ignores_the_requested_variant() {
    let plain = lifecycle_request(
        LifecycleOperation::Content { variant: None },
        "task_1",
        Some("https://api.test/v1"),
    )
    .unwrap();
    let chosen = lifecycle_request(
        LifecycleOperation::Content {
            variant: Some("thumbnail"),
        },
        "task_1",
        Some("https://api.test/v1"),
    )
    .unwrap();
    assert_eq!(plain, chosen);
}

#[test]
fn cancel_reply_reports_cancelled_and_keeps_the_raw_id() {
    let body = r#"{"id":"task_1","status":"RUNNING","output":["https://cdn.test/a.mp4"],"progress":0.5,"failure":"boom","createdAt":"1970-01-01T00:00:05Z"}"#;
    let video = cancel_response(body.as_bytes()).unwrap();
    assert_eq!(video.id, "task_1");
    assert_eq!(video.object, VideoObjectType::Video);
    assert_eq!(video.status, "cancelled");
    assert_eq!(video.created_at, Some(5));
    assert_eq!(video.completed_at, None);
    assert_eq!(video.progress, None);
    assert!(video.error.is_none());
    assert!(video.usage.is_none());
    assert!(video.model.is_none());
    assert!(video.size.is_none());
    assert!(video.seconds.is_none());
}

#[test]
fn cancel_reply_zeroes_an_absent_or_unreadable_created_at() {
    let absent = cancel_response(br#"{"id":"task_1"}"#).unwrap();
    assert_eq!(absent.created_at, Some(0));
    let null = cancel_response(br#"{"id":"task_1","createdAt":null}"#).unwrap();
    assert_eq!(null.created_at, Some(0));
    let unreadable = cancel_response(br#"{"id":"task_1","createdAt":"yesterday"}"#).unwrap();
    assert_eq!(unreadable.created_at, Some(0));
    let empty = cancel_response(b"{}").unwrap();
    assert_eq!(empty.id, "");
    assert_eq!(empty.created_at, Some(0));
}

#[rstest]
#[case::zero(0.0_f64, 0)]
#[case::eighth(0.125_f64, 12)]
#[case::three_eighths(0.375_f64, 38)]
#[case::half(0.5_f64, 50)]
#[case::almost_one(0.999_f64, 100)]
#[case::one(1.0_f64, 100)]
#[case::beyond_one(1.5_f64, 100)]
#[case::negative(-0.2_f64, 0)]
#[case::tie_rounds_down(0.545_f64, 54)]
#[case::tie_rounds_up(0.575_f64, 58)]
fn retrieval_progress_is_half_even_percent_clamped_to_the_range(
    #[case] share: f64,
    #[case] expected: i64,
) {
    let body = format!(r#"{{"id":"t","status":"RUNNING","progress":{share}}}"#);
    let video = retrieve_response(body.as_bytes(), None).unwrap();
    assert_eq!(video.progress, Some(expected), "progress {share}");
}

#[test]
fn retrieval_leaves_progress_absent_without_a_number() {
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert_eq!(absent.progress, None);
    let null = retrieve_response(br#"{"id":"t","progress":null}"#, None).unwrap();
    assert_eq!(null.progress, None);
}

#[rstest]
#[case::succeeded("SUCCEEDED", "completed")]
#[case::running("RUNNING", "in_progress")]
#[case::failed("FAILED", "failed")]
#[case::cancelled("CANCELLED", "failed")]
#[case::pending("PENDING", "queued")]
#[case::throttled("THROTTLED", "queued")]
#[case::lowercase_succeeded("succeeded", "completed")]
#[case::unknown("weird", "queued")]
fn retrieval_normalizes_the_provider_status(#[case] status: &str, #[case] expected: &str) {
    let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
    let video = retrieve_response(body.as_bytes(), None).unwrap();
    assert_eq!(video.status, expected, "status {status}");
}

#[test]
fn retrieval_defaults_the_status_to_queued_when_absent_or_null() {
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert_eq!(absent.status, "queued");
    let null = retrieve_response(br#"{"id":"t","status":null}"#, None).unwrap();
    assert_eq!(null.status, "queued");
}

#[test]
fn retrieval_always_sets_the_creation_time() {
    let present =
        retrieve_response(br#"{"id":"t","createdAt":"1970-01-01T00:00:05Z"}"#, None).unwrap();
    assert_eq!(present.created_at, Some(5));
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert_eq!(absent.created_at, Some(0));
}

#[test]
fn retrieval_reports_completion_only_when_the_key_is_present() {
    let present =
        retrieve_response(br#"{"id":"t","completedAt":"1970-01-01T00:00:07Z"}"#, None).unwrap();
    assert_eq!(present.completed_at, Some(7));
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert_eq!(absent.completed_at, None);
    let null = retrieve_response(br#"{"id":"t","completedAt":null}"#, None).unwrap();
    assert_eq!(null.completed_at, Some(0));
    let unreadable = retrieve_response(br#"{"id":"t","completedAt":"soon"}"#, None).unwrap();
    assert_eq!(unreadable.completed_at, Some(0));
}

#[test]
fn retrieval_builds_the_error_from_present_failure_fields() {
    let both =
        retrieve_response(br#"{"id":"t","failure":"boom","failureCode":"E1"}"#, None).unwrap();
    let code = both.error.as_ref().and_then(|error| error.get("code"));
    assert_eq!(code.and_then(|value| value.as_str()), Some("E1"));
    let message = both.error.as_ref().and_then(|error| error.get("message"));
    assert_eq!(message.and_then(|value| value.as_str()), Some("boom"));
    let code_only = retrieve_response(br#"{"id":"t","failureCode":""}"#, None).unwrap();
    let only_code = code_only.error.as_ref().and_then(|error| error.get("code"));
    assert_eq!(only_code.and_then(|value| value.as_str()), Some(""));
    let only_message = code_only
        .error
        .as_ref()
        .and_then(|error| error.get("message"));
    assert_eq!(
        only_message.and_then(|value| value.as_str()),
        Some("Video generation failed")
    );
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert!(absent.error.is_none());
}

#[test]
fn retrieval_wraps_a_nonempty_id_without_the_model() {
    let wrapped = retrieve_response(br#"{"id":"task_1"}"#, Some("runwayml")).unwrap();
    assert_ne!(wrapped.id, "task_1");
    let decoded = decode_video_id_with_provider(&wrapped.id);
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("runwayml"));
    assert_eq!(decoded.model_id.as_deref(), Some(""));
    assert_eq!(decoded.video_id, "task_1");
    let empty = retrieve_response(br#"{"id":""}"#, Some("runwayml")).unwrap();
    assert_eq!(empty.id, "");
    let missing = retrieve_response(br#"{"status":"PENDING"}"#, Some("runwayml")).unwrap();
    assert_eq!(missing.id, "");
    let raw = retrieve_response(br#"{"id":"task_1"}"#, None).unwrap();
    assert_eq!(raw.id, "task_1");
}

#[test]
fn retrieval_adds_no_usage_or_request_fields() {
    let body = br#"{"id":"t","status":"SUCCEEDED","output":["https://cdn.test/a.mp4"],"estimatedCost":{"credits":4.0}}"#;
    let video = retrieve_response(body, None).unwrap();
    assert!(video.usage.is_none());
    assert!(video.model.is_none());
    assert!(video.size.is_none());
    assert!(video.seconds.is_none());
    assert!(video.remixed_from_video_id.is_none());
    assert!(video.expires_at.is_none());
    let reply = serde_json::to_string(&video).unwrap();
    assert!(!reply.contains("output_url"), "{reply}");
}

#[tokio::test]
async fn content_downloads_a_truthy_output_string_without_credentials() {
    let body = r#"{"id":"t","status":"RUNNING","output":"https://cdn.test/a.mp4"}"#;
    let script = Script::once(200, "video-bytes");
    let bytes = content_response(body.as_bytes(), &script, LIMITS, DEADLINE)
        .await
        .unwrap();
    assert_eq!(bytes.as_slice(), b"video-bytes".as_slice());
    let requests = script.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, "https://cdn.test/a.mp4");
    assert!(
        requests[0].headers.is_empty(),
        "the artifact request carries no provider credentials"
    );
    assert_eq!(requests[0].max_bytes, LIMITS.max_json_bytes);
}

#[tokio::test]
async fn content_takes_only_the_first_artifact_element() {
    let body = r#"{"id":"t","status":"SUCCEEDED","output":["https://cdn.test/first.mp4","https://cdn.test/second.mp4"]}"#;
    let script = Script::once(200, "first-bytes");
    let bytes = content_response(body.as_bytes(), &script, LIMITS, DEADLINE)
        .await
        .unwrap();
    assert_eq!(bytes.as_slice(), b"first-bytes".as_slice());
    assert_eq!(script.requests()[0].url, "https://cdn.test/first.mp4");
    let empty_first =
        r#"{"id":"t","status":"SUCCEEDED","output":["","https://cdn.test/second.mp4"]}"#;
    let error = content_response(empty_first.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), MISSING_URL);
}

#[rstest]
#[case::pending("PENDING")]
#[case::running("RUNNING")]
#[case::throttled("THROTTLED")]
#[tokio::test]
async fn content_keeps_the_exact_uppercase_processing_statuses(#[case] status: &str) {
    let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
    let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Video is still processing (status: {status}). Please wait and try again.")
    );
}

#[rstest]
#[case::lowercase("running")]
#[case::capitalized("Running")]
#[case::trailing_space("RUNNING ")]
#[tokio::test]
async fn content_does_not_treat_a_lowercase_running_status_as_still_processing(
    #[case] status: &str,
) {
    let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
    let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), MISSING_URL, "status {status}");
}

#[rstest]
#[case::failure_text(
    r#"{"id":"t","status":"FAILED","failure":"boom"}"#,
    "Video generation failed: boom"
)]
#[case::absent_failure(
    r#"{"id":"t","status":"FAILED"}"#,
    "Video generation failed: Unknown error"
)]
#[case::null_failure(
    r#"{"id":"t","status":"FAILED","failure":null}"#,
    "Video generation failed: Unknown error"
)]
#[case::empty_failure(
    r#"{"id":"t","status":"FAILED","failure":""}"#,
    "Video generation failed: "
)]
#[case::lowercase_failed(r#"{"id":"t","status":"failed","failure":"boom"}"#, MISSING_URL)]
#[case::cancelled(r#"{"id":"t","status":"CANCELLED"}"#, MISSING_URL)]
#[case::succeeded_without_output(r#"{"id":"t","status":"SUCCEEDED"}"#, MISSING_URL)]
#[case::absent_status(r#"{"id":"t"}"#, MISSING_URL)]
#[case::null_status(r#"{"id":"t","status":null}"#, MISSING_URL)]
#[tokio::test]
async fn content_reports_the_failure_text_or_unknown_error(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected, "body {body}");
}

#[rstest]
#[case::empty_string(r#"{"id":"t","status":"SUCCEEDED","output":""}"#)]
#[case::empty_list(r#"{"id":"t","status":"SUCCEEDED","output":[]}"#)]
#[case::null_output(r#"{"id":"t","status":"SUCCEEDED","output":null}"#)]
#[tokio::test]
async fn content_treats_empty_outputs_as_absent(#[case] body: &str) {
    let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), MISSING_URL, "body {body}");
}

#[rstest]
#[case::number_output(
    r#"{"id":"t","status":"SUCCEEDED","output":5}"#,
    "the output value must be a string or a list of strings"
)]
#[case::object_output(
    r#"{"id":"t","status":"SUCCEEDED","output":{"url":"https://cdn.test/a.mp4"}}"#,
    "the output value must be a string or a list of strings"
)]
#[case::object_first_element(
    r#"{"id":"t","status":"SUCCEEDED","output":[{"url":"https://cdn.test/a.mp4"}]}"#,
    "the output value must be a string or a list of strings"
)]
#[case::number_failure(
    r#"{"id":"t","status":"FAILED","failure":5}"#,
    "the failure value must be a string"
)]
#[tokio::test]
async fn content_refuses_output_and_failure_values_of_another_type(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected, "body {body}");
}

#[tokio::test]
async fn content_reports_an_artifact_http_failure_as_the_provider_error() {
    let body = r#"{"id":"t","status":"SUCCEEDED","output":["https://cdn.test/a.mp4"]}"#;
    let mut headers = HeaderMap::new();
    headers.insert("x-runway-trace", HeaderValue::from_static("abc"));
    let script = Script::new(vec![Reply {
        status: 404,
        body: b"not found".to_vec(),
        headers,
    }]);
    let outcome = content_response(body.as_bytes(), &script, LIMITS, DEADLINE).await;
    let Err(VideoLifecycleError::ArtifactStatus(error)) = outcome else {
        panic!("expected the artifact status error");
    };
    assert_eq!(error.status_code, 404);
    assert_eq!(error.message, "not found");
    assert_eq!(
        error.headers.get("x-runway-trace"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert_eq!(script.requests().len(), 1);
}

#[tokio::test]
async fn content_keeps_the_valid_prefix_of_a_non_utf8_failure_body() {
    let body = r#"{"id":"t","status":"SUCCEEDED","output":["https://cdn.test/a.mp4"]}"#;
    let script = Script::new(vec![Reply {
        status: 500,
        body: b"bad \xFF bytes".to_vec(),
        headers: HeaderMap::new(),
    }]);
    let outcome = content_response(body.as_bytes(), &script, LIMITS, DEADLINE).await;
    let Err(VideoLifecycleError::ArtifactStatus(error)) = outcome else {
        panic!("expected the artifact status error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "bad ");
    assert!(error.headers.is_empty());
}

#[tokio::test]
async fn content_reports_a_redirect_reply_as_the_provider_error() {
    let body = r#"{"id":"t","status":"SUCCEEDED","output":["https://cdn.test/a.mp4"]}"#;
    let script = Script::once(302, "moved");
    let outcome = content_response(body.as_bytes(), &script, LIMITS, DEADLINE).await;
    let Err(VideoLifecycleError::ArtifactStatus(error)) = outcome else {
        panic!("expected the artifact status error");
    };
    assert_eq!(error.status_code, 302);
    assert_eq!(error.message, "moved");
}

#[tokio::test]
async fn content_refuses_an_unparsable_artifact_url() {
    let body = r#"{"id":"t","status":"SUCCEEDED","output":["://not a url"]}"#;
    let outcome = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE).await;
    assert!(
        matches!(outcome, Err(VideoLifecycleError::ArtifactUrl(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn lifecycle_replies_refuse_a_body_that_is_not_a_task() {
    let outcome = content_response(b"not json", &Script::none(), LIMITS, DEADLINE).await;
    assert!(matches!(outcome, Err(VideoLifecycleError::Task(_))));
    assert!(matches!(
        retrieve_response(b"not json", None),
        Err(VideoLifecycleError::Task(_))
    ));
    assert!(matches!(
        cancel_response(b"not json"),
        Err(VideoLifecycleError::Task(_))
    ));
}

#[rstest]
#[case::remix(
    UnsupportedOperation::Remix,
    "Video remix is not yet supported by RunwayML API"
)]
#[case::list(
    UnsupportedOperation::List,
    "Video listing is not yet supported by RunwayML API"
)]
#[case::create_character(
    UnsupportedOperation::CreateCharacter,
    "video create character is not supported for RunwayML"
)]
#[case::get_character(
    UnsupportedOperation::GetCharacter,
    "video get character is not supported for RunwayML"
)]
#[case::edit(UnsupportedOperation::Edit, "video edit is not supported for RunwayML")]
#[case::extension(
    UnsupportedOperation::Extension,
    "video extension is not supported for RunwayML"
)]
fn unsupported_requests_keep_each_source_message(
    #[case] operation: UnsupportedOperation,
    #[case] expected: &str,
) {
    assert_eq!(operation.message(), expected);
    let Err(VideoLifecycleError::Unsupported { message }) = unsupported_request(operation) else {
        panic!("every unsupported request raises");
    };
    assert_eq!(message, expected);
}
