use betterlite_models::decisions::Json;

use crate::callback_logger::CallbackLogger as _;
use crate::posthog_logging::tests::support::{
    API_KEY, Reply, ScriptedTransport, capture_logs, default_settings, dynamic, event, event_with,
    field, fixed_id, items, list_len, logger_with, logger_with_check, payload, settings, settle,
    text, validating_check,
};

fn api_key(body: &Json) -> String {
    match field(body, "api_key") {
        Json::Text(key) => key.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the api key is text")
        }
    }
}

#[tokio::test]
async fn posthog_sync_send_posts_the_batch_immediately() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    logger
        .log_success_event(&event(payload()))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://us.i.posthog.com/batch/");
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(api_key(&requests[0].body), API_KEY);
    let batch = field(&requests[0].body, "batch");
    assert_eq!(list_len(batch), 1);
    assert_eq!(field(&items(batch)[0], "event"), &text("$ai_generation"));
    assert_eq!(field(&items(batch)[0], "distinct_id"), &text("trace-1"));
    assert_eq!(logger.sink.queued_depth(), 0);
}

#[tokio::test]
async fn posthog_sync_send_uses_the_per_request_credentials() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let params = dynamic(&[
        ("posthog_api_key", "request-key"),
        ("posthog_api_url", "https://request.example///"),
    ]);
    logger
        .log_success_event(&event_with(payload(), params))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    let requests = transport.requests();
    assert_eq!(requests[0].url, "https://request.example/batch/");
    assert_eq!(api_key(&requests[0].body), "request-key");
}

#[tokio::test]
async fn posthog_sync_send_swallows_a_failed_status() {
    let transport = ScriptedTransport::new(&[Reply::status(500)]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let result = logger.log_success_event(&event(payload()));
    result.expect("the sync send reports no error");
    settle(&tasks).await;
    assert_eq!(transport.urls().len(), 1);
}

#[tokio::test]
async fn posthog_sync_send_swallows_a_transport_failure() {
    let transport = ScriptedTransport::new(&[Reply::network()]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let result = logger.log_success_event(&event(payload()));
    result.expect("the sync send reports no error");
    settle(&tasks).await;
    assert_eq!(transport.urls().len(), 1);
}

#[tokio::test]
async fn posthog_sync_send_swallows_an_unbuildable_event() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    let mut broken = payload();
    broken.metadata.requester_metadata = Some(text("not an object"));
    let result = logger.log_success_event(&event(broken));
    result.expect("the sync send reports no error");
    settle(&tasks).await;
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn posthog_sync_send_mocks_a_posthog_host() {
    let transport = ScriptedTransport::new(&[]);
    let settings = settings(&[
        ("POSTHOG_API_KEY", API_KEY),
        ("POSTHOG_API_URL", "https://us.i.posthog.com"),
        ("POSTHOG_MOCK", "true"),
    ]);
    let (logger, tasks) = logger_with(settings, &transport, fixed_id);
    logger
        .log_success_event(&event(payload()))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn posthog_sync_send_leaves_a_custom_url_live_in_mock_mode() {
    let transport = ScriptedTransport::new(&[]);
    let settings = settings(&[
        ("POSTHOG_API_KEY", API_KEY),
        ("POSTHOG_API_URL", "http://localhost:8080"),
        ("POSTHOG_MOCK", "true"),
    ]);
    let (logger, tasks) = logger_with(settings, &transport, fixed_id);
    logger
        .log_success_event(&event(payload()))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    assert_eq!(transport.urls(), vec!["http://localhost:8080/batch/"]);
}

#[tokio::test]
async fn posthog_sync_send_refuses_a_request_url_on_a_private_host() {
    let (_guard, logs) = capture_logs();
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) =
        logger_with_check(default_settings(), &transport, validating_check(), fixed_id);
    let params = dynamic(&[
        ("posthog_api_key", "request-key"),
        ("posthog_api_url", "https://private.example"),
    ]);
    logger
        .log_success_event(&event_with(payload(), params))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    assert!(transport.requests().is_empty());
    assert!(logs.contents().contains("blocked address (127.0.0.1)"));
}

#[tokio::test]
async fn posthog_sync_send_uses_a_request_url_on_a_public_host() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) =
        logger_with_check(default_settings(), &transport, validating_check(), fixed_id);
    let params = dynamic(&[
        ("posthog_api_key", "request-key"),
        ("posthog_api_url", "https://public.example"),
    ]);
    logger
        .log_success_event(&event_with(payload(), params))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://public.example/batch/");
    assert_eq!(api_key(&requests[0].body), "request-key");
}

#[tokio::test]
async fn posthog_sync_send_posts_a_plain_request_url_to_the_validated_address() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) =
        logger_with_check(default_settings(), &transport, validating_check(), fixed_id);
    let params = dynamic(&[
        ("posthog_api_key", "request-key"),
        ("posthog_api_url", "http://public.example"),
    ]);
    logger
        .log_success_event(&event_with(payload(), params))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://8.8.8.8/batch/");
    assert_eq!(
        requests[0].headers.get("host").map(String::as_str),
        Some("public.example")
    );
}
