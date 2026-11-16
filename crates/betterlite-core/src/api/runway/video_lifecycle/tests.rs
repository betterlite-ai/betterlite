#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::reply::VideoObjectType;
use http::HeaderMap;
use http::header::HeaderValue;
use serde_json::json;

use super::artifact::ArtifactTransport;
use super::cancel::{cancel_request, cancel_response};
use super::content::{content_request, content_response};
use super::error::VideoLifecycleError;
use super::retrieve::{retrieve_request, retrieve_response};
use super::unsupported::{UnsupportedOperation, unsupported_request};
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
    let content =
        content_request(&wrapped, Some("https://api.test/v1/"), Some("thumbnail")).unwrap();
    assert_eq!(content.url, "https://api.test/v1/tasks/task%2Fone%20two");
    assert!(content.data.is_empty());
    let retrieve = retrieve_request(&wrapped, Some("https://api.test/v1")).unwrap();
    assert_eq!(retrieve.url, "https://api.test/v1/tasks/task%2Fone%20two");
    assert!(retrieve.data.is_empty());
    let cancel = cancel_request(&wrapped, Some("https://api.test/v1")).unwrap();
    assert_eq!(
        cancel.url,
        "https://api.test/v1/tasks/task%2Fone%20two/cancel"
    );
    assert!(cancel.data.is_empty());
    assert_eq!(
        retrieve_request("task_1", None).unwrap().url,
        "https://api.dev.runwayml.com/v1/tasks/task_1"
    );
}

#[test]
fn content_request_ignores_the_requested_variant() {
    let plain = content_request("task_1", Some("https://api.test/v1"), None).unwrap();
    let chosen = content_request("task_1", Some("https://api.test/v1"), Some("thumbnail")).unwrap();
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

#[test]
fn retrieval_progress_is_half_even_percent_clamped_to_the_range() {
    let cases = [
        (0.0, 0),
        (0.125, 12),
        (0.375, 38),
        (0.5, 50),
        (0.999, 100),
        (1.0, 100),
        (1.5, 100),
        (-0.2, 0),
        (0.545, 54),
        (0.575, 58),
    ];
    for (share, expected) in cases {
        let body = format!(r#"{{"id":"t","status":"RUNNING","progress":{share}}}"#);
        let video = retrieve_response(body.as_bytes(), None).unwrap();
        assert_eq!(video.progress, Some(expected), "progress {share}");
    }
}

#[test]
fn retrieval_leaves_progress_absent_without_a_number() {
    let absent = retrieve_response(br#"{"id":"t"}"#, None).unwrap();
    assert_eq!(absent.progress, None);
    let null = retrieve_response(br#"{"id":"t","progress":null}"#, None).unwrap();
    assert_eq!(null.progress, None);
}

#[test]
fn retrieval_normalizes_the_provider_status() {
    let cases = [
        ("SUCCEEDED", "completed"),
        ("RUNNING", "in_progress"),
        ("FAILED", "failed"),
        ("CANCELLED", "failed"),
        ("PENDING", "queued"),
        ("THROTTLED", "queued"),
        ("succeeded", "completed"),
        ("weird", "queued"),
    ];
    for (status, expected) in cases {
        let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
        let video = retrieve_response(body.as_bytes(), None).unwrap();
        assert_eq!(video.status, expected, "status {status}");
    }
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
    assert_eq!(
        both.error.as_ref().and_then(|error| error.get("code")),
        Some(&json!("E1"))
    );
    assert_eq!(
        both.error.as_ref().and_then(|error| error.get("message")),
        Some(&json!("boom"))
    );
    let code_only = retrieve_response(br#"{"id":"t","failureCode":""}"#, None).unwrap();
    assert_eq!(
        code_only.error.as_ref().and_then(|error| error.get("code")),
        Some(&json!(""))
    );
    assert_eq!(
        code_only
            .error
            .as_ref()
            .and_then(|error| error.get("message")),
        Some(&json!("Video generation failed"))
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
    assert_eq!(decoded.model_id, None);
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
    let reply = serde_json::to_value(&video).unwrap();
    assert!(reply.get("output_url").is_none(), "{reply}");
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

#[tokio::test]
async fn content_keeps_the_exact_uppercase_processing_statuses() {
    for status in ["PENDING", "RUNNING", "THROTTLED"] {
        let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
        let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("Video is still processing (status: {status}). Please wait and try again.")
        );
    }
}

#[tokio::test]
async fn content_does_not_treat_a_lowercase_running_status_as_still_processing() {
    for status in ["running", "Running", "RUNNING "] {
        let body = format!(r#"{{"id":"t","status":"{status}"}}"#);
        let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), MISSING_URL, "status {status}");
    }
}

#[tokio::test]
async fn content_reports_the_failure_text_or_unknown_error() {
    let cases = [
        (
            r#"{"id":"t","status":"FAILED","failure":"boom"}"#,
            "Video generation failed: boom",
        ),
        (
            r#"{"id":"t","status":"FAILED"}"#,
            "Video generation failed: Unknown error",
        ),
        (
            r#"{"id":"t","status":"FAILED","failure":null}"#,
            "Video generation failed: Unknown error",
        ),
        (
            r#"{"id":"t","status":"FAILED","failure":""}"#,
            "Video generation failed: ",
        ),
        (
            r#"{"id":"t","status":"failed","failure":"boom"}"#,
            MISSING_URL,
        ),
        (r#"{"id":"t","status":"CANCELLED"}"#, MISSING_URL),
        (r#"{"id":"t","status":"SUCCEEDED"}"#, MISSING_URL),
        (r#"{"id":"t"}"#, MISSING_URL),
        (r#"{"id":"t","status":null}"#, MISSING_URL),
    ];
    for (body, expected) in cases {
        let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected, "body {body}");
    }
}

#[tokio::test]
async fn content_treats_empty_outputs_as_absent() {
    let cases = [
        r#"{"id":"t","status":"SUCCEEDED","output":""}"#,
        r#"{"id":"t","status":"SUCCEEDED","output":[]}"#,
        r#"{"id":"t","status":"SUCCEEDED","output":null}"#,
    ];
    for body in cases {
        let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), MISSING_URL, "body {body}");
    }
}

#[tokio::test]
async fn content_refuses_output_and_failure_values_of_another_type() {
    let output_cases = [
        r#"{"id":"t","status":"SUCCEEDED","output":5}"#,
        r#"{"id":"t","status":"SUCCEEDED","output":{"url":"https://cdn.test/a.mp4"}}"#,
        r#"{"id":"t","status":"SUCCEEDED","output":[{"url":"https://cdn.test/a.mp4"}]}"#,
    ];
    for body in output_cases {
        let error = content_response(body.as_bytes(), &Script::none(), LIMITS, DEADLINE)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the output value must be a string or a list of strings",
            "body {body}"
        );
    }
    let failure = r#"{"id":"t","status":"FAILED","failure":5}"#;
    let error = content_response(failure.as_bytes(), &Script::none(), LIMITS, DEADLINE)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "the failure value must be a string");
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
    assert_eq!(error.headers.get("x-runway-trace"), Some(&json!("abc")));
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
async fn content_refuses_an_unparseable_artifact_url() {
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

#[test]
fn unsupported_requests_keep_each_source_message() {
    let cases = [
        (
            UnsupportedOperation::Remix,
            "Video remix is not yet supported by RunwayML API",
        ),
        (
            UnsupportedOperation::List,
            "Video listing is not yet supported by RunwayML API",
        ),
        (
            UnsupportedOperation::CreateCharacter,
            "video create character is not supported for RunwayML",
        ),
        (
            UnsupportedOperation::GetCharacter,
            "video get character is not supported for RunwayML",
        ),
        (
            UnsupportedOperation::Edit,
            "video edit is not supported for RunwayML",
        ),
        (
            UnsupportedOperation::Extension,
            "video extension is not supported for RunwayML",
        ),
    ];
    for (operation, expected) in cases {
        assert_eq!(operation.message(), expected);
        let Err(VideoLifecycleError::Unsupported { message }) = unsupported_request(operation)
        else {
            panic!("every unsupported request raises");
        };
        assert_eq!(message, expected);
    }
}
