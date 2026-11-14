#![cfg(test)]

use std::error::Error as _;

use rstest::rstest;

use super::SpendLogsFetcher;
use crate::commands::debug::error::DebugError;
use crate::commands::debug::wire::SpendLogRow;
use crate::commands::tests::support::{Answer, TestServer, client, client_for};

fn page_body(rows: &str, total_pages: i64) -> String {
    format!(r#"{{"data":[{rows}],"total":0,"total_pages":{total_pages}}}"#)
}

fn rows_from(json: &str) -> Vec<SpendLogRow> {
    serde_json::from_str(json).unwrap()
}

fn null_answers(count: usize) -> Vec<Answer> {
    (0..count).map(|_| Answer::json(200, "null")).collect()
}

#[test]
fn fetch_asks_for_the_first_page_of_the_session() {
    let server = TestServer::start(vec![Answer::json(200, &page_body("", 1))]);
    let client = client_for(&server, None);

    SpendLogsFetcher { client: &client }
        .session_rows("sess-1")
        .unwrap();

    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), "GET");
    assert_eq!(
        requests[0].path(),
        "/spend/logs/session/ui?session_id=sess-1&page=1&page_size=100"
    );
}

#[test]
fn fetch_percent_encodes_the_session_id_in_the_query() {
    let server = TestServer::start(vec![Answer::json(200, &page_body("", 1))]);
    let client = client_for(&server, None);

    SpendLogsFetcher { client: &client }
        .session_rows("a&b=c")
        .unwrap();

    assert_eq!(
        server.finish()[0].path(),
        "/spend/logs/session/ui?session_id=a%26b%3Dc&page=1&page_size=100"
    );
}

#[test]
fn fetch_sends_the_api_key_as_a_bearer_token() {
    let server = TestServer::start(vec![Answer::json(200, &page_body("", 1))]);
    let client = client_for(&server, Some("sk-test"));

    SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap();

    assert_eq!(
        server.finish()[0].header("authorization"),
        Some("Bearer sk-test")
    );
}

#[rstest]
#[case::one_page(1, 1)]
#[case::no_pages(0, 1)]
#[case::negative_pages(-4, 1)]
#[case::three_pages(3, 3)]
fn fetch_reads_every_page_the_first_one_announces(
    #[case] total_pages: i64,
    #[case] expected_requests: usize,
) {
    let answers = (0..expected_requests)
        .map(|index| {
            Answer::json(
                200,
                &page_body(&format!(r#"{{"request_id":"r{index}"}}"#), total_pages),
            )
        })
        .collect();
    let server = TestServer::start(answers);
    let client = client_for(&server, None);

    let rows = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap();

    let requests = server.finish();
    assert_eq!(requests.len(), expected_requests);
    assert_eq!(rows.len(), expected_requests);
}

#[test]
fn fetch_requests_the_pages_in_order() {
    let answers = (1..=3)
        .map(|index| {
            Answer::json(
                200,
                &page_body(&format!(r#"{{"request_id":"r{index}"}}"#), 3),
            )
        })
        .collect();
    let server = TestServer::start(answers);
    let client = client_for(&server, None);

    SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap();

    let paths: Vec<String> = server
        .finish()
        .iter()
        .map(|request| request.path().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/spend/logs/session/ui?session_id=s&page=1&page_size=100",
            "/spend/logs/session/ui?session_id=s&page=2&page_size=100",
            "/spend/logs/session/ui?session_id=s&page=3&page_size=100",
        ]
    );
}

#[test]
fn fetch_sorts_the_rows_of_all_pages_by_start_time() {
    let first = page_body(
        r#"{"request_id":"late","startTime":"2026-01-01T00:00:09"},{"request_id":"none"}"#,
        2,
    );
    let second = page_body(
        r#"{"request_id":"early","startTime":"2026-01-01T00:00:01"}"#,
        2,
    );
    let server = TestServer::start(vec![Answer::json(200, &first), Answer::json(200, &second)]);
    let client = client_for(&server, None);

    let rows = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap();

    server.finish();
    let ids: Vec<&str> = rows.iter().map(|row| row.request_id.as_str()).collect();
    assert_eq!(ids, ["none", "early", "late"]);
}

#[test]
fn fetch_keeps_the_arrival_order_of_rows_with_the_same_start_time() {
    let body = page_body(
        r#"{"request_id":"b","startTime":"t"},{"request_id":"a","startTime":"t"},{"request_id":"c","startTime":"t"}"#,
        1,
    );
    let server = TestServer::start(vec![Answer::json(200, &body)]);
    let client = client_for(&server, None);

    let rows = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap();

    server.finish();
    let ids: Vec<&str> = rows.iter().map(|row| row.request_id.as_str()).collect();
    assert_eq!(ids, ["b", "a", "c"]);
}

#[test]
fn fetch_stops_at_the_first_failing_page() {
    let first = page_body(r#"{"request_id":"a"}"#, 3);
    let server = TestServer::start(vec![Answer::json(200, &first), Answer::text(500, "boom")]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    assert_eq!(server.finish().len(), 2);
    assert_eq!(
        error.to_string(),
        "GET /spend/logs/session/ui failed with HTTP 500: boom"
    );
}

#[rstest]
#[case::just_below_the_threshold(399, true)]
#[case::at_the_threshold(400, false)]
#[case::server_error(503, false)]
fn fetch_treats_http_400_and_above_as_a_failure(#[case] status: u16, #[case] succeeds: bool) {
    let server = TestServer::start(vec![Answer::json(status, &page_body("", 1))]);
    let client = client_for(&server, None);

    let outcome = SpendLogsFetcher { client: &client }.session_rows("s");

    server.finish();
    assert_eq!(outcome.is_ok(), succeeds);
}

#[rstest]
#[case::short("short body", "short body")]
#[case::empty("", "")]
fn fetch_puts_the_response_body_in_the_status_error(#[case] body: &str, #[case] expected: &str) {
    let server = TestServer::start(vec![Answer::text(502, body)]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    server.finish();
    assert!(matches!(
        &error,
        DebugError::Status { uri, status: 502, body } if uri == "/spend/logs/session/ui" && body == expected
    ));
}

#[rstest]
#[case::at_the_limit(500, 500)]
#[case::one_over(501, 500)]
#[case::one_under(499, 499)]
fn fetch_cuts_the_status_body_after_500_characters(
    #[case] length: usize,
    #[case] expected_length: usize,
) {
    let body = "\u{e9}".repeat(length);
    let server = TestServer::start(vec![Answer::text(500, &body)]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    server.finish();
    let DebugError::Status { body, .. } = error else {
        panic!("expected a status error");
    };
    assert_eq!(body, "\u{e9}".repeat(expected_length));
}

#[test]
fn fetch_drops_bytes_that_are_not_text_from_the_status_body() {
    let mut answer = Answer::text(500, "");
    answer.body = vec![b'o', 0xff, b'k'];
    let server = TestServer::start(vec![answer]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    server.finish();
    assert!(matches!(&error, DebugError::Status { body, .. } if body == "ok"));
}

#[rstest]
#[case::missing_fields("{}")]
#[case::not_json("<html>")]
#[case::list("[]")]
fn fetch_names_the_uri_of_a_response_with_an_unexpected_shape(#[case] body: &str) {
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    server.finish();
    assert_eq!(
        error.to_string(),
        "Unexpected /spend/logs/session/ui response"
    );
    assert!(error.source().is_some());
}

#[test]
fn fetch_reports_a_proxy_that_cannot_be_reached() {
    let client = client("http://127.0.0.1:1", None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    assert!(matches!(&error, DebugError::Request { uri, .. } if uri == "/spend/logs/session/ui"));
}

#[test]
fn fetch_reports_a_base_url_that_is_not_a_url() {
    let client = client("not a url", None);

    let error = SpendLogsFetcher { client: &client }
        .session_rows("s")
        .unwrap_err();

    assert!(matches!(&error, DebugError::Request { uri, .. } if uri == "/spend/logs/session/ui"));
}

#[rstest]
#[case::plain("req-1", "/spend/logs/ui/req-1")]
#[case::slash("a/b", "/spend/logs/ui/a%2Fb")]
#[case::space("a b", "/spend/logs/ui/a%20b")]
#[case::question_mark("a?b", "/spend/logs/ui/a%3Fb")]
#[case::hash("a#b", "/spend/logs/ui/a%23b")]
fn fetch_puts_the_request_id_in_one_path_segment(#[case] request_id: &str, #[case] path: &str) {
    let server = TestServer::start(vec![Answer::json(200, "null")]);
    let client = client_for(&server, None);

    SpendLogsFetcher { client: &client }
        .payload(request_id)
        .unwrap();

    assert_eq!(server.finish()[0].path(), path);
}

#[test]
fn fetch_reads_the_request_and_response_of_a_payload() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"proxy_server_request":{"model":"m"},"response":"done","messages":[1]}"#,
    )]);
    let client = client_for(&server, None);

    let payload = SpendLogsFetcher { client: &client }
        .payload("r")
        .unwrap()
        .unwrap();

    server.finish();
    assert_eq!(
        (
            payload.proxy_server_request.text(),
            payload.response.text(),
            payload.messages.text()
        ),
        (
            "{\n  \"model\": \"m\"\n}".to_owned(),
            "done".to_owned(),
            "[\n  1\n]".to_owned()
        )
    );
}

#[test]
fn fetch_reads_a_null_payload_as_none() {
    let server = TestServer::start(vec![Answer::json(200, "null")]);
    let client = client_for(&server, None);

    let payload = SpendLogsFetcher { client: &client }.payload("r").unwrap();

    server.finish();
    assert!(payload.is_none());
}

#[test]
fn fetch_names_the_payload_uri_in_a_status_error() {
    let server = TestServer::start(vec![Answer::text(404, "gone")]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .payload("req-9")
        .unwrap_err();

    server.finish();
    assert_eq!(
        error.to_string(),
        "GET /spend/logs/ui/req-9 failed with HTTP 404: gone"
    );
}

#[test]
fn fetch_reports_a_base_url_that_cannot_hold_a_path() {
    let client = client("mailto:ops@example.com", None);

    let error = SpendLogsFetcher { client: &client }
        .payload("r")
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "the proxy URL mailto:ops@example.com/spend/logs/ui cannot hold the path /spend/logs/ui/r"
    );
}

#[rstest]
#[case::failed_and_last_one(
    r#"[{"request_id":"a"},{"request_id":"b","status":"failure"},{"request_id":"c"},{"request_id":"d"}]"#,
    1,
    &["b", "d"]
)]
#[case::no_recent_bodies(
    r#"[{"request_id":"a"},{"request_id":"b","status":"failure"},{"request_id":"c"}]"#,
    0,
    &["b"]
)]
#[case::no_recent_bodies_and_no_failure(r#"[{"request_id":"a"},{"request_id":"b"}]"#, 0, &[])]
#[case::two_recent(
    r#"[{"request_id":"a"},{"request_id":"b","status":"failure"},{"request_id":"c"},{"request_id":"d"}]"#,
    2,
    &["b", "c", "d"]
)]
#[case::exactly_all(r#"[{"request_id":"a"},{"request_id":"b"}]"#, 2, &["a", "b"])]
#[case::more_than_all(r#"[{"request_id":"a"},{"request_id":"b"}]"#, 9, &["a", "b"])]
#[case::failed_inside_the_recent_window(
    r#"[{"request_id":"a"},{"request_id":"b","status":"failure"}]"#,
    1,
    &["b"]
)]
#[case::failed_by_metadata(
    r#"[{"request_id":"a","metadata":{"status":"failure"}},{"request_id":"b"},{"request_id":"c"}]"#,
    1,
    &["a", "c"]
)]
#[case::same_id_twice(r#"[{"request_id":"a"},{"request_id":"a"}]"#, 2, &["a"])]
#[case::fetched_in_id_order(
    r#"[{"request_id":"z"},{"request_id":"y"},{"request_id":"x"}]"#,
    3,
    &["x", "y", "z"]
)]
#[case::no_rows("[]", 3, &[])]
fn fetch_loads_the_payloads_of_failed_and_recent_rows(
    #[case] rows: &str,
    #[case] recent_bodies: usize,
    #[case] expected: &[&str],
) {
    let server = TestServer::start(null_answers(expected.len()));
    let client = client_for(&server, None);

    let payloads = SpendLogsFetcher { client: &client }
        .payloads(&rows_from(rows), recent_bodies)
        .unwrap();

    let paths: Vec<String> = server
        .finish()
        .iter()
        .map(|request| request.path().to_owned())
        .collect();
    let wanted_paths: Vec<String> = expected
        .iter()
        .map(|id| format!("/spend/logs/ui/{id}"))
        .collect();
    let loaded: Vec<&str> = payloads.keys().map(String::as_str).collect();
    assert_eq!(paths, wanted_paths);
    assert_eq!(loaded, expected);
}

#[test]
fn fetch_keeps_each_payload_under_its_request_id() {
    let server = TestServer::start(vec![
        Answer::json(200, r#"{"response":"for a"}"#),
        Answer::json(200, "null"),
    ]);
    let client = client_for(&server, None);

    let payloads = SpendLogsFetcher { client: &client }
        .payloads(&rows_from(r#"[{"request_id":"a"},{"request_id":"b"}]"#), 2)
        .unwrap();

    server.finish();
    assert_eq!(payloads["a"].as_ref().unwrap().response.text(), "for a");
    assert!(payloads["b"].is_none());
}

#[test]
fn fetch_stops_loading_payloads_at_the_first_failure() {
    let server = TestServer::start(vec![Answer::json(200, "null"), Answer::text(500, "boom")]);
    let client = client_for(&server, None);

    let error = SpendLogsFetcher { client: &client }
        .payloads(
            &rows_from(r#"[{"request_id":"a"},{"request_id":"b"},{"request_id":"c"}]"#),
            3,
        )
        .unwrap_err();

    assert_eq!(server.finish().len(), 2);
    assert_eq!(
        error.to_string(),
        "GET /spend/logs/ui/b failed with HTTP 500: boom"
    );
}
