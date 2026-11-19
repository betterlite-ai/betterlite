use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::error::{SsrfError, UserUrlError};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use http::HeaderValue;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

use super::super::upload::{
    DEFAULT_CONTENT_TYPE, FileData, HttpUrlFetch, MAX_FETCHED_FILE_BYTES, UploadError,
    content_type_from_headers, filename_from_url,
};
use super::*;

#[tokio::test]
async fn upload_precedence_prefers_file_data_over_url_and_id() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(
            Some(FileData {
                filename: "inline.txt".to_owned(),
                content: b"inline".to_vec(),
                content_type: "text/plain".to_owned(),
            }),
            Some("https://host/remote.txt"),
            Some("file_1"),
        )
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some("inline.txt"));
    assert_eq!(uploaded.file_content.as_deref(), Some(b"inline".as_slice()));
    assert_eq!(uploaded.content_type.as_deref(), Some("text/plain"));
    assert!(uploaded.existing_file_id.is_none());
    assert!(harness.fetch.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upload_precedence_uses_the_url_without_file_data() {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        200,
        header("content-type", "text/plain"),
        b"remote",
    ));
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some("https://host/folder/remote.txt"), Some("file_1"))
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some("remote.txt"));
    assert_eq!(uploaded.file_content.as_deref(), Some(b"remote".as_slice()));
    assert!(uploaded.existing_file_id.is_none());
    let calls = harness.fetch.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "https://host/folder/remote.txt");
}

#[tokio::test]
async fn upload_precedence_falls_back_to_the_file_id() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, None, Some("file_123"))
        .await
        .expect("the upload");
    assert_eq!(uploaded.existing_file_id.as_deref(), Some("file_123"));
    assert!(uploaded.filename.is_none());
    assert!(uploaded.file_content.is_none());
    assert!(uploaded.content_type.is_none());
    assert!(harness.fetch.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn upload_precedence_rejects_an_absent_source() {
    let state = Harness::default().state(IngestionOptions::default());
    let error = state
        .upload(None, None, None)
        .await
        .expect_err("the source is required");
    assert!(matches!(error, UploadError::MissingSource));
}

#[tokio::test]
async fn upload_precedence_fetches_an_explicit_empty_url() {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some(""), Some("file_1"))
        .await
        .expect("the upload");
    assert!(uploaded.existing_file_id.is_none());
    assert_eq!(uploaded.filename.as_deref(), Some("document"));
    let calls = harness.fetch.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "");
}

#[rstest]
#[case::file_in_directory("https://host/dir/report.pdf", "report.pdf")]
#[case::trailing_slash("https://host/dir/", "document")]
#[case::bare_name("report.pdf", "report.pdf")]
#[tokio::test]
async fn upload_fetch_derives_the_filename_from_the_url_path(
    #[case] url: &str,
    #[case] expected: &str,
) {
    let harness = Harness::default();
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some(url), None)
        .await
        .expect("the upload");
    assert_eq!(uploaded.filename.as_deref(), Some(expected));
}

#[test]
fn upload_fetch_takes_the_content_type_from_the_response_headers() {
    assert_eq!(
        content_type_from_headers(&header("content-type", "application/pdf; charset=binary")),
        "application/pdf; charset=binary"
    );
    assert_eq!(content_type_from_headers(&header("content-type", "")), "");
    assert_eq!(
        content_type_from_headers(&HeaderMap::new()),
        DEFAULT_CONTENT_TYPE
    );
}

#[test]
fn upload_fetch_defaults_the_filename_of_an_empty_url() {
    assert_eq!(filename_from_url(""), "document");
}

#[tokio::test]
async fn upload_fetch_reports_a_failed_status() {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        404,
        HeaderMap::new(),
        b"missing",
    ));
    let state = harness.state(IngestionOptions::default());
    let error = state
        .upload(None, Some("https://host/missing.txt"), None)
        .await
        .expect_err("the status fails the upload");
    assert!(matches!(error, UploadError::HttpStatus { status: 404 }));
}

#[rstest]
#[case::ok(200)]
#[case::last_success(299)]
#[tokio::test]
async fn upload_fetch_accepts_a_2xx_status(#[case] status: u16) {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        status,
        HeaderMap::new(),
        b"body",
    ));
    let state = harness.state(IngestionOptions::default());
    let uploaded = state
        .upload(None, Some("https://host/file.txt"), None)
        .await
        .expect("the upload succeeds");
    assert_eq!(uploaded.file_content.as_deref(), Some(b"body".as_slice()));
}

#[rstest]
#[case::multiple_choices(300)]
#[case::redirect(302)]
#[case::last_redirection(399)]
#[tokio::test]
async fn upload_fetch_rejects_a_non_2xx_status(#[case] status: u16) {
    let harness = Harness::default().with_fetch(FakeFetch::default().reply(
        status,
        HeaderMap::new(),
        b"redirected",
    ));
    let state = harness.state(IngestionOptions::default());
    let error = state
        .upload(None, Some("https://host/file.txt"), None)
        .await
        .expect_err("the status fails the upload");
    assert!(matches!(
        error,
        UploadError::HttpStatus { status: failed } if failed == status
    ));
}

#[tokio::test]
async fn upload_fetch_propagates_a_port_error() {
    let harness =
        Harness::default().with_fetch(FakeFetch::default().failing("the host is blocked"));
    let state = harness.state(IngestionOptions::default());
    let error = state
        .upload(None, Some("https://host/file.txt"), None)
        .await
        .expect_err("the fetch fails");
    assert!(matches!(error, UploadError::Fetch(_)));
}

fn header(name: &'static str, value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(name, HeaderValue::from_static(value));
    headers
}

fn http_fetch(allowed_hosts: &[String]) -> HttpUrlFetch<SystemResolver> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    let client = ProxiedClient::build(
        base_builder().http1_only(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .expect("the client");
    HttpUrlFetch {
        client,
        urls: UserUrlFetcher {
            validator: UrlValidator::new(SystemResolver, allowed_hosts, &Verification::Enabled),
            validation: true,
        },
    }
}

#[tokio::test]
async fn upload_fetch_reads_the_body_and_headers_through_the_checked_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/files/report.txt");
    let fetch = http_fetch(&["127.0.0.1".to_owned()]);
    let serving = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 1024];
        let _read = socket.read(&mut buffer).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(fetch.fetch(&url), serving);
    let response = result.expect("the fetch succeeds");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"hello");
    assert_eq!(content_type_from_headers(&response.headers), "text/plain");
}

#[tokio::test]
async fn upload_fetch_refuses_a_body_over_the_download_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/files/huge.bin");
    let fetch = http_fetch(&["127.0.0.1".to_owned()]);
    let declared = MAX_FETCHED_FILE_BYTES.saturating_add(1);
    let serving = async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0_u8; 1024];
        let _read = socket.read(&mut buffer).await.unwrap();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
    };
    let (result, ()) = tokio::join!(fetch.fetch(&url), serving);
    let error = result.expect_err("the body is refused");
    let UrlFetchError::Read(source) = error else {
        panic!("the body is refused as a read error");
    };
    assert!(matches!(
        *source,
        TransportError::BodyTooLarge { limit } if limit == MAX_FETCHED_FILE_BYTES
    ));
}

#[tokio::test]
async fn upload_fetch_refuses_a_blocked_address() {
    let fetch = http_fetch(&[]);
    let error = fetch
        .fetch("http://127.0.0.1:9/file.txt")
        .await
        .expect_err("the address is blocked");
    let UrlFetchError::Request(source) = error else {
        panic!("the address is refused as a request error");
    };
    assert!(matches!(
        *source,
        UserUrlError::Ssrf(SsrfError::BlockedAddress { .. })
    ));
}
