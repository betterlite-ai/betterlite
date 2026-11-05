#![cfg(test)]

use std::io;

use rstest::rstest;

use super::MAX_FILE_BYTES;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t"}"#;

fn client(transport: &FakeTransport) -> GitLabClient<FakeTransport> {
    GitLabClient::new(&config_from(CONFIG), transport.clone()).unwrap()
}

#[tokio::test]
async fn a_raw_hit_returns_the_body_and_makes_one_request() {
    let transport = FakeTransport::scripted(vec![answer!(200, "Hello {{ name }}")]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some("Hello {{ name }}"));
    assert_eq!(transport.requests().len(), 1);
    assert!(transport.requests()[0].url.contains("/raw?ref=main"));
}

#[tokio::test]
async fn an_empty_raw_body_is_an_empty_string_not_missing() {
    let transport = FakeTransport::scripted(vec![answer!(200, "")]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some(""));
}

#[tokio::test]
async fn invalid_utf8_in_a_raw_body_is_replaced() {
    let transport = FakeTransport::scripted(vec![Ok(GitLabResponse {
        status: 200,
        body: b"ok \xff end".to_vec(),
    })]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some("ok \u{FFFD} end"));
}

#[tokio::test]
async fn a_raw_404_falls_back_to_the_json_endpoint_and_decodes_base64() {
    let transport = FakeTransport::scripted(vec![
        answer!(404, ""),
        answer!(
            200,
            r#"{"content": "SGVsbG8gd29ybGQ=", "encoding": "base64"}"#,
        ),
    ]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some("Hello world"));
    let urls: Vec<String> = transport
        .requests()
        .into_iter()
        .map(|request| request.url)
        .collect();
    assert_eq!(urls.len(), 2);
    assert!(urls[0].ends_with("/a.prompt/raw?ref=main"));
    assert!(urls[1].ends_with("/a.prompt?ref=main"));
}

#[tokio::test]
async fn a_404_on_both_endpoints_is_none() {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(404, "")]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text, None);
}

#[rstest]
#[case::invalid_utf8_replaced(
    r#"{"content": "b2sg/yBlbmQ=", "encoding": "base64"}"#,
    "ok \u{FFFD} end"
)]
#[case::whitespace_in_base64_is_dropped(
    r#"{"content": "SGVs\nbG8g\r\nd29y bGQ=", "encoding": "base64"}"#,
    "Hello world"
)]
#[case::non_base64_encoding_is_returned_as_is(
    r#"{"content": "plain text", "encoding": "text"}"#,
    "plain text"
)]
#[case::missing_encoding_is_returned_as_is(r#"{"content": "SGk="}"#, "SGk=")]
#[case::empty_content_is_empty(r#"{"content": "", "encoding": "base64"}"#, "")]
#[tokio::test]
async fn the_json_fallback_decodes_the_content_field(#[case] body: &str, #[case] expected: &str) {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(200, body)]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text.as_deref(), Some(expected));
}

#[tokio::test]
async fn a_json_payload_without_content_is_none() {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(200, "{}")]);
    let text = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(text, None);
}

#[tokio::test]
async fn a_malformed_json_payload_is_a_json_endpoint_error() {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(200, "not json")]);
    let error = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap_err();
    assert!(matches!(error, GitLabClientError::JsonFetch { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Failed to fetch file 'a.prompt' via JSON endpoint: ")
    );
}

#[rstest]
#[case::unauthorized(401, "Authentication failed. Check your GitLab token and auth_method.")]
#[case::forbidden(
    403,
    "Access denied to file 'a.prompt'. Check your GitLab permissions for project 'g/r'."
)]
#[case::server_error(
    500,
    "Failed to fetch file 'a.prompt': GitLab answered with status 500"
)]
#[case::rate_limited(
    429,
    "Failed to fetch file 'a.prompt': GitLab answered with status 429"
)]
#[tokio::test]
async fn a_raw_error_status_has_its_own_message_and_never_falls_back(
    #[case] status: u16,
    #[case] message: &str,
) {
    let transport = FakeTransport::scripted(vec![answer!(status, ""), answer!(200, "{}")]);
    let error = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert_eq!(transport.requests().len(), 1);
}

#[rstest]
#[case::unauthorized(401, "Authentication failed. Check your GitLab token and auth_method.")]
#[case::forbidden(
    403,
    "Access denied to file 'a.prompt'. Check your GitLab permissions for project 'g/r'."
)]
#[case::server_error(
    502,
    "Failed to fetch file 'a.prompt' via JSON endpoint: GitLab answered with status 502"
)]
#[tokio::test]
async fn a_json_error_status_has_its_own_message(#[case] status: u16, #[case] message: &str) {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(status, "")]);
    let error = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[tokio::test]
async fn a_transport_failure_is_a_fetch_error_on_the_raw_endpoint() {
    let transport = FakeTransport::scripted(vec![Err(io::Error::other("connection reset"))]);
    let error = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap_err();
    assert!(matches!(error, GitLabClientError::Fetch { .. }));
    assert!(error.to_string().contains("connection reset"));
}

#[tokio::test]
async fn a_transport_failure_on_the_json_fallback_is_a_json_endpoint_error() {
    let transport =
        FakeTransport::scripted(vec![answer!(404, ""), Err(io::Error::other("timed out"))]);
    let error = client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap_err();
    assert!(matches!(error, GitLabClientError::JsonFetch { .. }));
}

#[tokio::test]
async fn every_request_carries_the_body_cap() {
    let transport = FakeTransport::scripted(vec![answer!(404, ""), answer!(404, "")]);
    client(&transport)
        .get_file_content("a.prompt", None)
        .await
        .unwrap();
    assert_eq!(MAX_FILE_BYTES, 0x0010_0000);
    assert!(
        transport
            .requests()
            .iter()
            .all(|request| request.max_body_bytes == MAX_FILE_BYTES)
    );
}
