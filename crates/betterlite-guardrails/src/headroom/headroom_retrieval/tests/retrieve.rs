use std::time::Duration;

use secrecy::SecretString;

use super::super::retrieve::fetch;
use super::support;

const PATH: &str = "/v1/retrieve/abcdef123456";
const FAILED: &str = "[Headroom: retrieval failed for hash=abcdef123456]";

#[tokio::test]
async fn the_original_content_is_returned() {
    let server = support::start(vec![support::retrieve_reply("full text")]).await;
    let harness = support::harness(&server.base);
    let content = fetch(&harness.guardrail, support::HASH, None).await;
    assert_eq!(content, "full text");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, PATH);
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
}

#[tokio::test]
async fn an_empty_original_content_is_returned() {
    let server = support::start(vec![support::reply(200, r#"{"original_content":""}"#)]).await;
    let harness = support::harness(&server.base);
    assert_eq!(fetch(&harness.guardrail, support::HASH, None).await, "");
}

#[tokio::test]
async fn the_query_is_sent_when_present() {
    let server = support::start(vec![support::retrieve_reply("ranked")]).await;
    let harness = support::harness(&server.base);
    let content = fetch(&harness.guardrail, support::HASH, Some("find it")).await;
    assert_eq!(content, "ranked");
    assert_eq!(
        server.requests()[0].path,
        "/v1/retrieve/abcdef123456?query=find+it"
    );
}

#[tokio::test]
async fn a_404_is_reported_as_expired() {
    let server = support::start(vec![support::reply(404, "gone")]).await;
    let harness = support::harness(&server.base);
    let content = fetch(&harness.guardrail, support::HASH, None).await;
    assert_eq!(
        content,
        "[Headroom: hash=abcdef123456 not found or expired]"
    );
}

#[tokio::test]
async fn another_status_is_reported_with_its_code() {
    let server = support::start(vec![support::reply(503, "busy")]).await;
    let harness = support::harness(&server.base);
    let content = fetch(&harness.guardrail, support::HASH, None).await;
    assert_eq!(
        content,
        "[Headroom: retrieval error 503 for hash=abcdef123456]"
    );
}

#[tokio::test]
async fn a_non_json_body_is_returned_as_text() {
    let server = support::start(vec![support::reply(200, "plain text")]).await;
    let harness = support::harness(&server.base);
    assert_eq!(
        fetch(&harness.guardrail, support::HASH, None).await,
        "plain text"
    );
}

#[tokio::test]
async fn json_without_original_content_is_returned_as_text() {
    let server = support::start(vec![support::reply(200, r#"{"other":1}"#)]).await;
    let harness = support::harness(&server.base);
    assert_eq!(
        fetch(&harness.guardrail, support::HASH, None).await,
        r#"{"other":1}"#
    );
}

#[tokio::test]
async fn a_non_string_original_content_is_returned_as_text() {
    let server = support::start(vec![support::reply(200, r#"{"original_content":5}"#)]).await;
    let harness = support::harness(&server.base);
    assert_eq!(
        fetch(&harness.guardrail, support::HASH, None).await,
        r#"{"original_content":5}"#
    );
}

#[tokio::test]
async fn a_transport_failure_is_reported() {
    let harness = support::harness("http://127.0.0.1:1");
    assert_eq!(fetch(&harness.guardrail, support::HASH, None).await, FAILED);
}

#[tokio::test]
async fn a_timeout_is_reported() {
    let server = support::start(vec![support::delayed(
        200,
        "late",
        Duration::from_millis(300),
    )])
    .await;
    let harness = support::harness_with_timeout(&server.base, Some(0.05_f64));
    assert_eq!(fetch(&harness.guardrail, support::HASH, None).await, FAILED);
}

#[tokio::test]
async fn unusable_headers_are_reported() {
    let server = support::start(vec![support::reply(200, "{}")]).await;
    let mut harness = support::harness(&server.base);
    harness.guardrail.settings.api_key = Some(SecretString::from("bad\nkey"));
    assert_eq!(fetch(&harness.guardrail, support::HASH, None).await, FAILED);
    assert!(server.requests().is_empty());
}
