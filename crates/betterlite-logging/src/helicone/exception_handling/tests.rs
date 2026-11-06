#![cfg(test)]

use betterlite_models::decisions::Json;
use tokio::join;

use crate::helicone::test_support::{capture_debug, event, json, logger, serve_once};

#[tokio::test]
async fn exception_handling_logs_success_and_posts_once() {
    let (captured, _guard) = capture_debug();
    let (base, server) = serve_once(200, "ok").await;
    let logger = logger(&base, Some("k"));
    let sent = event("gpt-4o");
    let ((), received) = join!(logger.log_success(&sent), server);
    assert_eq!(received.request_line, "POST /oai/v1/log HTTP/1.1");
    let output = captured.text();
    assert!(output.contains("Helicone Logging - Enters logging function for model"));
    assert!(output.contains("Helicone Logging - Success!"));
}

#[tokio::test]
async fn exception_handling_logs_a_rejected_request_with_its_body() {
    let (captured, _guard) = capture_debug();
    let (base, server) = serve_once(401, "bad key").await;
    let logger = logger(&base, Some("k"));
    let sent = event("gpt-4o");
    let ((), _received) = join!(logger.log_success(&sent), server);
    let output = captured.text();
    assert!(output.contains("Helicone Logging - Error Request was not successful"));
    assert!(output.contains("401"));
    assert!(output.contains("bad key"));
    assert!(!output.contains("Success!"));
}

#[tokio::test]
async fn exception_handling_swallows_an_unreachable_server() {
    let (captured, _guard) = capture_debug();
    logger("http://127.0.0.1:1", Some("k"))
        .log_success(&event("gpt-4o"))
        .await;
    assert!(captured.text().contains("Helicone Logging Error"));
}

#[tokio::test]
async fn exception_handling_skips_the_post_without_an_api_key() {
    let (captured, _guard) = capture_debug();
    logger("http://127.0.0.1:1", None)
        .log_success(&event("gpt-4o"))
        .await;
    let output = captured.text();
    assert!(output.contains("Helicone Logging Error"));
    assert!(output.contains("MissingKey"));
}

#[tokio::test]
async fn exception_handling_swallows_a_response_that_cannot_be_mapped() {
    let (captured, _guard) = capture_debug();
    let mut broken = event("claude-3-opus");
    broken.response = json(
        r#"{"id": "x", "choices": [], "usage": {"prompt_tokens": 1, "completion_tokens": 1}}"#,
    );
    logger("http://127.0.0.1:1", Some("k"))
        .log_success(&broken)
        .await;
    let output = captured.text();
    assert!(output.contains("Helicone Logging Error"));
    assert!(output.contains("NoChoices"));
}

#[tokio::test]
async fn exception_handling_swallows_a_non_completion_claude_response() {
    let (captured, _guard) = capture_debug();
    let mut broken = event("claude-3-opus");
    broken.response = Json::Text("not a completion".to_owned());
    logger("http://127.0.0.1:1", Some("k"))
        .log_success(&broken)
        .await;
    assert!(captured.text().contains("Helicone Logging Error"));
}
