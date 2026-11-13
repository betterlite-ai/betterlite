#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::config::BitBucketConfig;
use super::error::ClientError;
use super::path::sanitize_file_path;
use super::{BitBucketClient, DEFAULT_BASE_URL, content};

// nothing listens on port 1, so a request that reaches the network fails at once
const REQUIRED: &str = "workspace: ws\nrepository: repo\naccess_token: tok\n";

const CLOSED_BASE: &str = "http://127.0.0.1:1/2.0";

fn transport() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

fn config_from(yaml: &str) -> BitBucketConfig {
    serde_saphyr::from_str(yaml).unwrap()
}

fn build(yaml: &str) -> Result<BitBucketClient, ClientError> {
    BitBucketClient::new(config_from(yaml), transport())
}

fn with_extra(extra: &str) -> String {
    format!("{REQUIRED}{extra}\n")
}

fn with_base(base: &str) -> String {
    with_extra(&format!("\"\": \"{base}\""))
}

async fn serve(listener: TcpListener, status: &str, content_type: &str, body: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    String::from_utf8(head).unwrap()
}

async fn fetch(
    file_path: &str,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> (String, Result<Option<String>, ClientError>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/2.0", listener.local_addr().unwrap());
    let client = build(&with_base(&base)).unwrap();
    tokio::join!(
        serve(listener, status, content_type, body),
        client.get_file_content(file_path)
    )
}

#[rstest]
#[case::missing_workspace("repository: repo\naccess_token: tok\n")]
#[case::missing_repository("workspace: ws\naccess_token: tok\n")]
#[case::missing_token("workspace: ws\nrepository: repo\n")]
#[case::empty_workspace("workspace: ''\nrepository: repo\naccess_token: tok\n")]
#[case::empty_repository("workspace: ws\nrepository: ''\naccess_token: tok\n")]
#[case::empty_token("workspace: ws\nrepository: repo\naccess_token: ''\n")]
#[case::null_token("workspace: ws\nrepository: repo\naccess_token: null\n")]
#[tokio::test]
async fn config_requires_workspace_repository_and_token(#[case] yaml: &str) {
    let error = build(yaml).unwrap_err();
    assert_eq!(
        error.to_string(),
        "workspace, repository, and access_token are required"
    );
}

#[tokio::test]
async fn config_defaults_branch_to_main_and_base_url_to_the_bitbucket_api() {
    let client = build(REQUIRED).unwrap();
    assert_eq!(
        (client.branch.as_str(), client.base_url.as_str()),
        ("main", "https://api.bitbucket.org/2.0")
    );
    assert_eq!(DEFAULT_BASE_URL, client.base_url);
}

#[tokio::test]
async fn config_keeps_an_explicit_branch_and_tolerates_unknown_keys() {
    let client = build(&with_extra(
        "branch: release\nprompts_path: prompts\ntimeout: 3",
    ))
    .unwrap();
    assert_eq!(client.branch, "release");
    assert_eq!(client.workspace, "ws");
    assert_eq!(client.repository, "repo");
}

#[rstest]
#[case::default_is_bearer("", "Bearer tok")]
#[case::token_method_is_bearer("auth_method: token", "Bearer tok")]
#[case::token_method_ignores_username("auth_method: token\nusername: alice", "Bearer tok")]
#[case::unknown_method_is_bearer("auth_method: oauth\nusername: alice", "Bearer tok")]
#[case::method_is_case_sensitive("auth_method: Basic\nusername: alice", "Bearer tok")]
#[case::basic_encodes_username_and_token(
    "auth_method: basic\nusername: alice",
    "Basic YWxpY2U6dG9r"
)]
#[case::username_alone_is_bearer("username: alice", "Bearer tok")]
#[tokio::test]
async fn auth_header_follows_method_and_username(#[case] extra: &str, #[case] expected: &str) {
    let client = build(&with_extra(extra)).unwrap();
    assert_eq!(client.headers.get(AUTHORIZATION).unwrap(), expected);
}

#[rstest]
#[case::no_username("auth_method: basic")]
#[case::empty_username("auth_method: basic\nusername: ''")]
#[case::null_username("auth_method: basic\nusername: null")]
#[tokio::test]
async fn auth_config_basic_without_a_username_falls_back_to_bearer(#[case] extra: &str) {
    let client = build(&with_extra(extra)).unwrap();
    assert_eq!(client.headers.get(AUTHORIZATION).unwrap(), "Bearer tok");
}

#[tokio::test]
async fn auth_config_headers_are_json_and_the_credential_is_sensitive() {
    let client = build(REQUIRED).unwrap();
    assert_eq!(client.headers.get(ACCEPT).unwrap(), "application/json");
    assert_eq!(
        client.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert!(client.headers.get(AUTHORIZATION).unwrap().is_sensitive());
    assert!(!format!("{client:?}").contains("tok\""));
}

#[tokio::test]
async fn auth_config_rejects_a_token_that_cannot_be_a_header_value() {
    let error = build("workspace: ws\nrepository: repo\naccess_token: \"a\\nb\"\n").unwrap_err();
    assert!(matches!(error, ClientError::InvalidAuthorization(_)));
}

#[tokio::test]
async fn auth_sends_the_authorization_header_on_the_request() {
    let (head, result) = fetch("a.prompt", "200 OK", "text/plain", b"hi").await;
    assert_eq!(result.unwrap().as_deref(), Some("hi"));
    assert!(head.contains("authorization: Bearer tok\r\n"));
    assert!(head.contains("accept: application/json\r\n"));
}

#[tokio::test]
async fn base_url_empty_key_replaces_the_default_api_base() {
    let (head, result) = fetch("a.prompt", "200 OK", "text/plain", b"hi").await;
    assert_eq!(result.unwrap().as_deref(), Some("hi"));
    assert!(head.starts_with("GET /2.0/repositories/ws/repo/src/main/a.prompt HTTP/1.1\r\n"));
}

#[tokio::test]
async fn base_url_empty_key_is_the_only_override_the_documented_key_is_ignored() {
    let client = build(&with_extra("base_url: http://elsewhere.invalid/api")).unwrap();
    assert_eq!(client.base_url, DEFAULT_BASE_URL);
}

#[tokio::test]
async fn base_url_empty_key_wins_over_the_documented_key() {
    let yaml = format!(
        "{}base_url: http://elsewhere.invalid/api\n",
        with_base("http://example.invalid/override")
    );
    let client = build(&yaml).unwrap();
    assert_eq!(client.base_url, "http://example.invalid/override");
}

#[rstest]
#[case::leading_dotdot("../a", false)]
#[case::middle_dotdot("a/../b", false)]
#[case::trailing_dotdot("a/..", false)]
#[case::only_dotdot("..", false)]
#[case::every_dotdot("a/../b/../c", false)]
#[case::question_mark("a?b", true)]
#[case::hash("a#b", true)]
#[case::special_before_traversal("../a?b", true)]
fn path_rejects_traversal_and_url_special_characters(
    #[case] file_path: &str,
    #[case] special: bool,
) {
    let error = sanitize_file_path(file_path).unwrap_err();
    let expected = if special {
        format!("Invalid file path {file_path:?}: contains URL special characters")
    } else {
        format!("Invalid file path {file_path:?}: path traversal detected")
    };
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::plain("dir/file.prompt", "dir/file.prompt")]
#[case::space("my dir/x y.prompt", "my%20dir/x%20y.prompt")]
#[case::unreserved_stay("a_b-c.d~e", "a_b-c.d~e")]
#[case::dots_inside_a_segment("..a/b..", "..a/b..")]
#[case::empty_segments_stay("a//b", "a//b")]
#[case::percent_is_encoded("a%2Fb", "a%252Fb")]
#[case::plus_and_colon("a+b:c", "a%2Bb%3Ac")]
#[case::unicode("\u{e9}.prompt", "%C3%A9.prompt")]
fn path_quotes_each_segment_separately(#[case] file_path: &str, #[case] expected: &str) {
    assert_eq!(sanitize_file_path(file_path).unwrap(), expected);
}

#[tokio::test]
async fn path_in_the_request_line_is_quoted_per_segment() {
    let (head, _) = fetch("dir/my file.prompt", "200 OK", "text/plain", b"").await;
    assert!(
        head.starts_with("GET /2.0/repositories/ws/repo/src/main/dir/my%20file.prompt HTTP/1.1")
    );
}

#[tokio::test]
async fn path_traversal_is_refused_before_any_request() {
    let client = build(&with_base(CLOSED_BASE)).unwrap();
    let error = client.get_file_content("../secret").await.unwrap_err();
    assert!(matches!(error, ClientError::Path(_)));
}

#[rstest]
#[case::missing_file_is_none("404 Not Found", "None")]
#[case::forbidden_names_workspace_and_repository(
    "403 Forbidden",
    "Access denied to file 'a.prompt'. Check your BitBucket permissions for workspace 'ws' and repository 'repo'."
)]
#[case::unauthorized_asks_for_a_new_token(
    "401 Unauthorized",
    "Authentication failed. Check your BitBucket access token and permissions."
)]
#[case::server_error_keeps_the_status(
    "500 Internal Server Error",
    "Failed to fetch file 'a.prompt': BitBucket answered with status 500"
)]
#[case::bad_request_keeps_the_status(
    "400 Bad Request",
    "Failed to fetch file 'a.prompt': BitBucket answered with status 400"
)]
#[tokio::test]
async fn error_status_separates_missing_denied_unauthorized_and_other(
    #[case] status: &str,
    #[case] expected: &str,
) {
    let (_, result) = fetch("a.prompt", status, "text/plain", b"nope").await;
    let observed = result.map_or_else(|error| error.to_string(), |found| format!("{found:?}"));
    assert_eq!(observed, expected);
}

#[tokio::test]
async fn error_transport_failure_names_the_file() {
    let client = build(&with_base(CLOSED_BASE)).unwrap();
    let error = client.get_file_content("a.prompt").await.unwrap_err();
    assert_eq!(error.to_string(), "Error fetching file 'a.prompt'");
    assert!(matches!(error, ClientError::Transport { .. }));
}

#[tokio::test]
async fn error_text_body_that_is_not_utf8_is_refused() {
    let (_, result) = fetch("a.prompt", "200 OK", "text/plain", &[0xff, 0xfe]).await;
    let error = result.unwrap_err();
    assert_eq!(error.to_string(), "The file 'a.prompt' is not valid UTF-8");
}

#[rstest]
#[case::text_is_returned_as_is("text/plain", b"aGVsbG8=", "aGVsbG8=")]
#[case::text_with_charset_is_returned_as_is("text/plain; charset=utf-8", b"aGVsbG8=", "aGVsbG8=")]
#[case::octet_stream_is_decoded("application/octet-stream", b"aGVsbG8=", "hello")]
#[case::json_type_is_decoded("application/json", b"aGVsbG8=", "hello")]
#[case::decoded_utf8_is_kept("application/json", b"w6k=", "\u{e9}")]
#[case::plain_text_that_is_not_base64_stays("application/json", b"hello world!", "hello world!")]
#[case::decoded_bytes_that_are_not_utf8_fall_back("application/json", b"/w==", "/w==")]
#[tokio::test]
async fn content_type_decides_between_text_base64_and_the_raw_body(
    #[case] content_type: &str,
    #[case] body: &[u8],
    #[case] expected: &str,
) {
    let (_, result) = fetch("a.prompt", "200 OK", content_type, body).await;
    assert_eq!(result.unwrap().as_deref(), Some(expected));
}

#[rstest]
#[case::without_a_content_type_it_decodes(None, b"aGVsbG8=", "hello")]
#[case::text_prefix_is_case_sensitive(Some(&b"Text/plain"[..]), b"aGVsbG8=", "hello")]
#[case::newlines_are_skipped(Some(&b"application/json"[..]), b"aGVs\nbG8=", "hello")]
#[case::padding_may_be_complete(Some(&b"application/json"[..]), b"aGVsbA==", "hell")]
#[case::noise_between_trailing_pads_is_skipped(Some(&b"application/json"[..]), b"aGVsbG8=\n=", "hello")]
#[case::missing_padding_falls_back(Some(&b"application/json"[..]), b"aGVsbG8", "aGVsbG8")]
#[case::short_padding_falls_back(Some(&b"application/json"[..]), b"aGVsbA=", "aGVsbA=")]
#[case::data_after_padding_falls_back(Some(&b"application/json"[..]), b"aGVsbG8=junk", "aGVsbG8=junk")]
#[case::data_after_completed_padding_decodes(
    Some(&b"application/json"[..]),
    b"YWJj=ZGVm",
    "abcdef"
)]
#[case::padding_before_the_last_sextet_falls_back(Some(&b"application/json"[..]), b"AA=A", "AA=A")]
#[case::an_empty_body_stays_empty(Some(&b"application/json"[..]), b"", "")]
#[case::only_padding_decodes_to_nothing(Some(&b"application/json"[..]), b"====", "")]
#[case::a_lone_sextet_falls_back(Some(&b"application/json"[..]), b"a", "a")]
fn content_base64_follows_the_non_strict_binascii_rules(
    #[case] content_type: Option<&[u8]>,
    #[case] body: &[u8],
    #[case] expected: &str,
) {
    assert_eq!(content::decode_body(content_type, body).unwrap(), expected);
}

#[rstest]
fn replays_the_bitbucket_file_bytes_corpus(
    #[files("fuzz/corpus/bitbucket_file_bytes/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(content::decode_body(None, &bytes));
    drop(content::decode_body(Some(b"text/plain"), &bytes));
    if let Ok(file_path) = str::from_utf8(&bytes) {
        drop(sanitize_file_path(file_path));
    }
}
