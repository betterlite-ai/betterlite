use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio_util::task::TaskTracker;

use super::support::{
    Fixture, RecordingDelivery, Reply, SPAN_URL, TRACE_URL, call, capture_logs, minimal_kwargs,
    object, sample_kwargs, sample_response,
};
use crate::opik::delivery::error::OpikError;

#[tokio::test]
async fn sync_success_posts_the_trace_batch_then_the_span_batch() {
    let mut fixture = Fixture::new();
    fixture.options.workspace = Some("team");
    fixture.options.api_key = Some("secret-key");
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, TRACE_URL);
    assert_eq!(requests[1].url, SPAN_URL);
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(requests[0].headers.get("comet-workspace").unwrap(), "team");
    assert_eq!(
        requests[1].headers.get("authorization").unwrap(),
        "secret-key"
    );
    assert_eq!(requests[0].batch("traces").len(), 1);
    assert_eq!(requests[1].batch("spans").len(), 1);
}

#[tokio::test]
async fn sync_success_payloads_carry_the_built_records() {
    let fixture = Fixture::new();
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let requests = fixture.delivery.requests();
    let traces = requests[0].batch("traces");
    assert_eq!(traces[0]["name"], Json::Text("chat.completion".to_owned()));
    assert_eq!(
        traces[0]["project_name"],
        Json::Text("Default Project".to_owned())
    );
    assert_eq!(traces[0]["thread_id"], Json::Text("thread-1".to_owned()));
    let spans = requests[1].batch("spans");
    assert_eq!(spans[0]["trace_id"], traces[0]["id"]);
    assert_eq!(spans[0]["model"], Json::Text("gpt-4o".to_owned()));
    assert_eq!(spans[0]["type"], Json::Text("llm".to_owned()));
    assert_eq!(
        spans[0]["name"],
        Json::Text("gpt-4o_chat.completion_7".to_owned())
    );
    assert_eq!(
        spans[0]["usage"],
        Json::Object(object(
            r#"{"completion_tokens":4,"prompt_tokens":3,"total_tokens":7}"#
        ))
    );
}

#[tokio::test]
async fn sync_success_sends_the_null_optional_fields_litellm_puts_in_dict() {
    let fixture = Fixture::new();
    let logger = fixture.build(None);
    let kwargs = minimal_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    let requests = fixture.delivery.requests();
    let traces = requests[0].batch("traces");
    assert_eq!(traces[0]["thread_id"], Json::Null);
    let spans = requests[1].batch("spans");
    assert_eq!(spans[0]["parent_span_id"], Json::Null);
    assert_eq!(spans[0]["provider"], Json::Null);
    assert_eq!(spans[0]["total_cost"], Json::Null);
}

#[tokio::test]
async fn async_batches_drop_the_null_optional_fields_on_their_way_to_the_wire() {
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::new();
    fixture.options.batch_size = Some(2);
    let logger = fixture.build(Some(&tracker));
    let kwargs = minimal_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;

    let requests = fixture.delivery.requests();
    assert_eq!(requests.len(), 2);
    let traces = requests[0].batch("traces");
    assert!(!traces[0].contains_key("thread_id"));
    let spans = requests[1].batch("spans");
    assert!(!spans[0].contains_key("parent_span_id"));
    assert!(!spans[0].contains_key("provider"));
    assert!(!spans[0].contains_key("total_cost"));
}

#[rstest]
#[case::no_content(204, true)]
#[case::ok(200, false)]
#[case::accepted(202, false)]
#[case::redirect(302, false)]
#[case::bad_request(400, false)]
#[case::server_error(500, false)]
#[tokio::test]
async fn sync_send_accepts_only_a_204_status(#[case] status: u16, #[case] accepted: bool) {
    let fixture = Fixture::with_delivery(RecordingDelivery::scripted([Reply::Status(status)]));
    let logger = fixture.build(None);

    let outcome = logger
        .send_expecting_204(&logger.settings.trace_url, &Json::Null)
        .await;

    match outcome {
        Ok(()) => assert!(accepted, "status {status} must not be accepted"),
        Err(OpikError::UnexpectedStatus {
            status: reported, ..
        }) => {
            assert!(!accepted, "status {status} must be accepted");
            assert_eq!(reported, status);
        }
        Err(other) => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn sync_send_reports_a_transport_failure() {
    let fixture = Fixture::with_delivery(RecordingDelivery::scripted([Reply::Transport]));
    let logger = fixture.build(None);

    let outcome = logger
        .send_expecting_204(&logger.settings.trace_url, &Json::Null)
        .await;

    assert!(matches!(outcome, Err(OpikError::Transport(_))));
}

#[tokio::test]
async fn sync_failure_on_the_trace_still_sends_the_span() {
    let (logs, _guard) = capture_logs();
    let fixture = Fixture::with_delivery(RecordingDelivery::scripted([
        Reply::Transport,
        Reply::Status(204),
    ]));
    let logger = fixture.build(None);
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger.log_success_event(call(&kwargs, &response)).await;

    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
    assert!(logs.contents().contains("OpikLogger failed to send batch"));
}

#[tokio::test]
async fn async_submit_logs_a_rejected_status_without_raising() {
    let (logs, _guard) = capture_logs();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::with_delivery(RecordingDelivery::scripted([
        Reply::Status(302),
        Reply::Status(500),
    ]));
    fixture.options.batch_size = Some(2);
    let logger = fixture.build(Some(&tracker));
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;

    let contents = logs.contents();
    assert!(contents.contains("status=302"));
    assert!(contents.contains("status=500"));
    assert!(contents.contains("rejected"));
    let sent = fixture.delivery.requests().len();
    logger.flush_queue().await;
    assert_eq!(fixture.delivery.requests().len(), sent);
}

#[tokio::test]
async fn async_submit_accepts_a_two_hundred_and_logs_the_event_count() {
    let (logs, _guard) = capture_logs();
    let tracker = TaskTracker::new();
    let mut fixture = Fixture::with_delivery(RecordingDelivery::scripted([
        Reply::Status(200),
        Reply::Status(204),
    ]));
    fixture.options.batch_size = Some(2);
    let logger = fixture.build(Some(&tracker));
    let kwargs = sample_kwargs();
    let response = sample_response();

    logger
        .async_log_success_event(call(&kwargs, &response))
        .await;

    assert_eq!(fixture.delivery.urls(), [TRACE_URL, SPAN_URL]);
    let contents = logs.contents();
    assert!(contents.contains("Opik events submitted"));
    assert!(contents.contains("events=2"));
}
