#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use rstest::rstest;
use tokio::time::advance;

use super::{BodyOutcome, parse_metadata_body};
use crate::mcp::oauth_protected::tests::support::{
    Harness, PASS_THROUGH_HOST, PASS_THROUGH_PATH, UPSTREAM_DOCUMENT, document, passthrough_server,
    rendered,
};

const JSON_ACCEPT: &str = "application/json";

#[tokio::test]
async fn fetches_the_host_candidate_first() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let initial = root
        .fetch_upstream_oauth_protected_resource(&passthrough_server())
        .await
        .expect("the fetch succeeds")
        .expect("the host candidate answers");
    assert_eq!(initial, document(UPSTREAM_DOCUMENT));
    assert_eq!(rendered(&initial), UPSTREAM_DOCUMENT);
    assert_eq!(
        root.upstream.calls(),
        vec![(String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),]
    );
}

#[tokio::test]
async fn falls_back_to_the_path_candidate() {
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&passthrough_server())
        .await
        .expect("the fetch succeeds")
        .expect("the path candidate answers");
    assert_eq!(fetched, document(UPSTREAM_DOCUMENT));
    assert_eq!(
        root.upstream.calls(),
        vec![
            (String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),
            (String::from(PASS_THROUGH_PATH), String::from(JSON_ACCEPT)),
        ]
    );
}

#[tokio::test]
async fn trims_trailing_slashes_from_the_path_candidate() {
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = Some(String::from("https://upstream.example.test/mcp/"));
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch succeeds")
        .expect("the trimmed path candidate answers");
    assert_eq!(fetched, document(UPSTREAM_DOCUMENT));
    assert_eq!(
        root.upstream.calls(),
        vec![
            (String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),
            (String::from(PASS_THROUGH_PATH), String::from(JSON_ACCEPT)),
        ]
    );
}

#[tokio::test]
async fn a_double_slash_path_asks_the_same_candidate_twice() {
    let harness = Harness::new().status(PASS_THROUGH_HOST, 404);
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = Some(String::from("https://upstream.example.test//"));
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch succeeds");
    assert!(fetched.is_none());
    assert_eq!(
        root.upstream.calls(),
        vec![
            (String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),
            (String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),
        ]
    );
}

#[tokio::test]
async fn a_url_without_a_path_asks_only_the_host_candidate() {
    let harness = Harness::new().status(PASS_THROUGH_HOST, 404);
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = Some(String::from("https://upstream.example.test"));
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch succeeds");
    assert!(fetched.is_none());
    assert_eq!(
        root.upstream.calls(),
        vec![(String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),]
    );
}

#[tokio::test]
async fn the_root_path_asks_only_the_host_candidate() {
    let harness = Harness::new().status(PASS_THROUGH_HOST, 404);
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = Some(String::from("https://upstream.example.test/"));
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch succeeds");
    assert!(fetched.is_none());
    assert_eq!(
        root.upstream.calls(),
        vec![(String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),]
    );
}

async fn calls_for_url(url: &str) -> Vec<(String, String)> {
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = Some(url.to_owned());
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch succeeds");
    assert_eq!(fetched, Some(document(UPSTREAM_DOCUMENT)));
    root.upstream.calls()
}

#[tokio::test]
async fn a_query_string_or_fragment_is_not_part_of_the_path_candidate() {
    let expected = vec![
        (String::from(PASS_THROUGH_HOST), String::from(JSON_ACCEPT)),
        (String::from(PASS_THROUGH_PATH), String::from(JSON_ACCEPT)),
    ];
    assert_eq!(
        calls_for_url("https://upstream.example.test/mcp?x=1").await,
        expected
    );
    assert_eq!(
        calls_for_url("https://upstream.example.test/mcp#y").await,
        expected
    );
}

#[tokio::test]
async fn skips_a_non_200_answer_even_with_a_document_body() {
    let harness = Harness::new()
        .status_body(
            PASS_THROUGH_HOST,
            404,
            r#"{"resource":"https://wrong.example.test"}"#,
        )
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&passthrough_server())
        .await
        .expect("the fetch succeeds")
        .expect("the path candidate answers");
    assert_eq!(fetched, document(UPSTREAM_DOCUMENT));
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test]
async fn skips_a_body_that_is_not_json() {
    let harness = Harness::new()
        .status_body(PASS_THROUGH_HOST, 200, "not json")
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&passthrough_server())
        .await
        .expect("the fetch succeeds")
        .expect("the path candidate answers");
    assert_eq!(fetched, document(UPSTREAM_DOCUMENT));
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test]
async fn skips_a_json_body_that_is_not_an_object() {
    let harness = Harness::new()
        .status_body(PASS_THROUGH_HOST, 200, "[1,2]")
        .reply(PASS_THROUGH_PATH, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&passthrough_server())
        .await
        .expect("the fetch succeeds")
        .expect("the path candidate answers");
    assert_eq!(fetched, document(UPSTREAM_DOCUMENT));
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn caches_a_hit_for_three_hundred_seconds() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let server = passthrough_server();
    let initial = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the first fetch succeeds")
        .expect("the first fetch gets the document");
    assert_eq!(rendered(&initial), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 1);
    advance(Duration::from_secs(299)).await;
    let cached = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the cached fetch succeeds")
        .expect("the hit keeps the document");
    assert_eq!(rendered(&cached), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 1);
    advance(Duration::from_secs(1)).await;
    let renewed = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the refetch succeeds")
        .expect("the refetch gets the document again");
    assert_eq!(rendered(&renewed), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn caches_a_miss_for_sixty_seconds() {
    let harness = Harness::new()
        .status(PASS_THROUGH_HOST, 404)
        .status(PASS_THROUGH_PATH, 404);
    let root = harness.root();
    let server = passthrough_server();
    assert!(
        root.fetch_upstream_oauth_protected_resource(&server)
            .await
            .expect("the first fetch succeeds")
            .is_none()
    );
    assert_eq!(root.upstream.calls().len(), 2);
    advance(Duration::from_secs(59)).await;
    assert!(
        root.fetch_upstream_oauth_protected_resource(&server)
            .await
            .expect("the cached fetch succeeds")
            .is_none()
    );
    assert_eq!(root.upstream.calls().len(), 2);
    advance(Duration::from_secs(1)).await;
    assert!(
        root.fetch_upstream_oauth_protected_resource(&server)
            .await
            .expect("the refetch succeeds")
            .is_none()
    );
    assert_eq!(root.upstream.calls().len(), 4);
}

#[tokio::test]
async fn raises_the_last_transport_failure_and_caches_nothing() {
    let harness = Harness::new()
        .transport_failure(PASS_THROUGH_HOST, "connection refused")
        .transport_failure(PASS_THROUGH_PATH, "tls handshake failed");
    let root = harness.root();
    let server = passthrough_server();
    let first = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect_err("every candidate failed on transport");
    assert_eq!(first.to_string(), "tls handshake failed");
    assert_eq!(root.upstream.calls().len(), 2);
    let second = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect_err("the failure is not cached");
    assert_eq!(second.to_string(), "tls handshake failed");
    assert_eq!(root.upstream.calls().len(), 4);
}

#[tokio::test]
async fn a_mixed_failure_caches_the_negative_entry() {
    let harness = Harness::new()
        .transport_failure(PASS_THROUGH_HOST, "connection refused")
        .other_failure(PASS_THROUGH_PATH, "invalid header");
    let root = harness.root();
    let server = passthrough_server();
    assert!(
        root.fetch_upstream_oauth_protected_resource(&server)
            .await
            .expect("a non-transport failure is not an error")
            .is_none()
    );
    assert_eq!(root.upstream.calls().len(), 2);
    assert!(
        root.fetch_upstream_oauth_protected_resource(&server)
            .await
            .expect("the cached miss succeeds")
            .is_none()
    );
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test]
async fn a_refused_store_still_answers_the_caller() {
    let harness = Harness::new()
        .reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT)
        .invalidate_on_fetch("srv-passthrough");
    let root = harness.root();
    let server = passthrough_server();
    let first = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the first fetch succeeds")
        .expect("the answer is returned even when it is not cached");
    assert_eq!(rendered(&first), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 1);
    let second = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the second fetch succeeds")
        .expect("the second fetch asks again");
    assert_eq!(rendered(&second), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 2);
}

#[tokio::test]
async fn reads_the_cache_after_taking_the_slot() {
    let harness = Harness::new().reply(PASS_THROUGH_HOST, UPSTREAM_DOCUMENT);
    let root = harness.root();
    let server = passthrough_server();
    let (first_result, second_result) = tokio::join!(
        root.fetch_upstream_oauth_protected_resource(&server),
        root.fetch_upstream_oauth_protected_resource(&server),
    );
    let first = first_result
        .expect("the first caller succeeds")
        .expect("the first caller gets the document");
    let second = second_result
        .expect("the second caller succeeds")
        .expect("the second caller reuses the cached document");
    assert_eq!(rendered(&first), UPSTREAM_DOCUMENT);
    assert_eq!(rendered(&second), UPSTREAM_DOCUMENT);
    assert_eq!(root.upstream.calls().len(), 1);
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
#[case::no_scheme(Some("upstream.example.test/mcp"))]
#[case::no_host(Some("https:///mcp"))]
#[case::empty_scheme(Some("://upstream.example.test/mcp"))]
#[tokio::test]
async fn a_url_without_a_scheme_or_host_asks_nothing(#[case] url: Option<&str>) {
    let harness = Harness::new();
    let root = harness.root();
    let mut server = passthrough_server();
    server.fields.url = url.map(str::to_owned);
    let fetched = root
        .fetch_upstream_oauth_protected_resource(&server)
        .await
        .expect("the fetch does not fail");
    assert!(fetched.is_none());
    assert!(root.upstream.calls().is_empty());
}

#[rstest]
#[case::null(b"null")]
#[case::boolean(b"true")]
#[case::number(b"1.5")]
#[case::text(b"\"text\"")]
#[case::list(b"[{\"a\":1}]")]
fn rejects_each_non_object_json_value(#[case] body: &[u8]) {
    assert!(matches!(parse_metadata_body(body), BodyOutcome::NotObject));
}

#[rstest]
#[case::empty(b"")]
#[case::truncated(b"{\"resource\":")]
#[case::not_utf8(b"{\"resource\":\"\xff\"}")]
#[case::nan_literal(b"{\"resource\":NaN}")]
#[case::utf16_le(b"\xff\xfe{\x00}\x00")]
fn reports_each_undecodable_body(#[case] body: &[u8]) {
    assert!(matches!(parse_metadata_body(body), BodyOutcome::NotJson(_)));
}

#[test]
fn decodes_a_nested_object_and_keeps_its_key_order() {
    let body = br#"{"z":[{"b":true,"a":null}],"a":"text"}"#;
    let BodyOutcome::Object(decoded) = parse_metadata_body(body) else {
        panic!("the fixture decodes as an object");
    };
    assert_eq!(
        rendered(&decoded),
        r#"{"z":[{"b":true,"a":null}],"a":"text"}"#
    );
}

#[rstest]
fn replays_the_protected_resource_body_corpus(
    #[files("fuzz/corpus/protected_resource_body/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus seed reads");
    let _outcome = parse_metadata_body(&bytes);
}
