#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::ocr_document::{
    InlineDocument, OcrDocument, inline_remote_document, validate_inline_document,
};
use crate::ocr_error::{OcrError, OcrTransportError};
use crate::ocr_settings::OcrSettings;

use crate::ocr_http::tests::support::{
    MockServer, chunked, connection, delayed, ocr_client, reply,
};

fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn settings(max_download_bytes: u64) -> OcrSettings {
    OcrSettings {
        request_timeout: Duration::from_mins(100),
        max_download_bytes,
        poll_timeout: Duration::from_mins(2),
        doc_intelligence_api_version: "2024-11-30".to_owned(),
        doc_intelligence_dpi: 96,
        vertex_project: None,
        vertex_location: None,
        enable_azure_ad_token_refresh: false,
    }
}

#[rstest]
#[case::pdf("data:application/pdf;base64,YWJj", b"abc")]
#[case::uppercase("DATA:application/pdf;BASE64,YWI", b"ab")]
#[case::percent_decoded("data:,a%20b%00%FF", b"a b\0\xff")]
fn decodes_data_urls_and_limits_decoded_size(#[case] source: &str, #[case] expected: &[u8]) {
    let inline = InlineDocument::parse(source)
        .expect("the data URI parses")
        .expect("the source is a data URI");
    assert_eq!(
        inline.decode(expected.len()).expect("within the cap"),
        expected
    );
    assert!(matches!(
        inline.decode(expected.len().saturating_sub(1)),
        Err(OcrError::InlineDocumentTooLarge)
    ));
}

#[test]
fn preserves_mime_parameters_and_standard_default() {
    let inline = InlineDocument::parse("data:application/pdf;version=1.7;base64,YQ==")
        .expect("the data URI parses")
        .expect("the source is a data URI");
    assert!(inline.0.mime_type().matches("application", "pdf"));
    assert_eq!(inline.0.mime_type().get_parameter("version"), Some("1.7"));
    let default = InlineDocument::parse("data:,a")
        .expect("the data URI parses")
        .expect("the source is a data URI");
    assert!(default.0.mime_type().matches("text", "plain"));
    assert_eq!(
        default.0.mime_type().get_parameter("charset"),
        Some("US-ASCII")
    );
}

#[rstest]
#[case::not_a_data_url("https://example.com/document.pdf")]
#[case::no_comma("data:application/pdf;base64")]
#[case::bad_base64("data:application/pdf;base64,INVALID!")]
fn rejects_invalid_inline_documents(#[case] source: &str) {
    assert!(matches!(
        validate_inline_document(&document(source)),
        Err(OcrError::InvalidDataUri)
    ));
}

#[tokio::test]
async fn remote_conversion_returns_a_data_uri_without_provider_credentials() {
    let server = MockServer::start(vec![reply(
        "200 OK",
        &[("content-type", "image/png; charset=binary")],
        b"abc",
    )])
    .await;
    let client = ocr_client(settings(50 * 1024 * 1024), Duration::from_secs(5));
    let mut extra = BTreeMap::new();
    let _previous = extra.insert("detail".to_owned(), Some("high".to_owned()));
    let source = OcrDocument::ImageUrl {
        image_url: format!("{}/image", server.base),
        extra,
    };
    let converted =
        inline_remote_document(&client, source, &connection(settings(50 * 1024 * 1024)))
            .await
            .expect("the image downloads");
    assert_eq!(converted.source(), "data:image/png;base64,YWJj");
    let OcrDocument::ImageUrl {
        extra: converted_extra,
        ..
    } = converted
    else {
        panic!("the document keeps its kind");
    };
    assert_eq!(
        converted_extra.get("detail"),
        Some(&Some("high".to_owned()))
    );
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    let lowered = requests[0].to_ascii_lowercase();
    assert!(!lowered.contains("authorization:"), "{}", requests[0]);
    assert!(!lowered.contains("api-key:"), "{}", requests[0]);
}

#[tokio::test]
async fn inline_documents_are_validated_and_returned_unchanged() {
    let client = ocr_client(settings(50 * 1024 * 1024), Duration::from_secs(5));
    let inline = document("data:application/pdf;base64,YWJj");
    let converted = inline_remote_document(&client, inline.clone(), &connection(settings(0)))
        .await
        .expect("an inline document is not downloaded");
    assert_eq!(converted, inline);
    let broken = document("data:application/pdf;base64,INVALID!");
    assert!(matches!(
        inline_remote_document(&client, broken, &connection(settings(0))).await,
        Err(OcrError::InvalidDataUri)
    ));
}

#[tokio::test]
async fn a_zero_download_limit_disables_documents() {
    let client = ocr_client(settings(0), Duration::from_secs(5));
    let remote = document("http://127.0.0.1:9/document");
    assert!(matches!(
        inline_remote_document(&client, remote, &connection(settings(0))).await,
        Err(OcrError::DownloadDisabled)
    ));
}

#[tokio::test]
async fn an_oversized_download_is_download_too_large() {
    let declared = MockServer::start(vec![reply("200 OK", &[], b"abc")]).await;
    let client = ocr_client(settings(2), Duration::from_secs(5));
    let declared_document = document(&format!("{}/document", declared.base));
    assert!(matches!(
        inline_remote_document(&client, declared_document, &connection(settings(2))).await,
        Err(OcrError::DownloadTooLarge)
    ));

    let streamed = MockServer::start(vec![chunked(&[b"ab", b"cd"])]).await;
    let streamed_document = document(&format!("{}/document", streamed.base));
    assert!(matches!(
        inline_remote_document(&client, streamed_document, &connection(settings(2))).await,
        Err(OcrError::DownloadTooLarge)
    ));
}

#[tokio::test]
async fn a_blocked_redirect_target_is_blocked_document_url() {
    let server = MockServer::start(vec![reply(
        "302 Found",
        &[("location", "http://10.0.0.1/document")],
        b"",
    )])
    .await;
    let client = ocr_client(settings(1024), Duration::from_secs(5));
    let remote = document(&format!("{}/document", server.base));
    assert!(matches!(
        inline_remote_document(&client, remote, &connection(settings(1024))).await,
        Err(OcrError::BlockedDocumentUrl)
    ));
}

#[tokio::test]
async fn a_redirect_without_location_is_missing_redirect_location() {
    let server = MockServer::start(vec![reply("302 Found", &[], b"")]).await;
    let client = ocr_client(settings(1024), Duration::from_secs(5));
    let remote = document(&format!("{}/document", server.base));
    assert!(matches!(
        inline_remote_document(&client, remote, &connection(settings(1024))).await,
        Err(OcrError::MissingRedirectLocation)
    ));
}

#[tokio::test]
async fn an_invalid_redirect_is_invalid_redirect() {
    let server = MockServer::start(vec![reply(
        "302 Found",
        &[("location", "http://[::1")],
        b"",
    )])
    .await;
    let client = ocr_client(settings(1024), Duration::from_secs(5));
    let remote = document(&format!("{}/document", server.base));
    assert!(matches!(
        inline_remote_document(&client, remote, &connection(settings(1024))).await,
        Err(OcrError::InvalidRedirect)
    ));
}

#[tokio::test]
async fn too_many_redirects_are_reported() {
    let redirect = reply("302 Found", &[("location", "/again")], b"");
    let server = MockServer::start(vec![redirect; 10]).await;
    let client = ocr_client(settings(1024), Duration::from_secs(5));
    let remote = document(&format!("{}/document", server.base));
    assert!(matches!(
        inline_remote_document(&client, remote, &connection(settings(1024))).await,
        Err(OcrError::TooManyRedirects)
    ));
}

#[tokio::test]
async fn an_error_status_is_transport_http() {
    let server = MockServer::start(vec![reply("500 Internal Server Error", &[], b"boom")]).await;
    let client = ocr_client(settings(1024), Duration::from_secs(5));
    let remote = document(&format!("{}/document", server.base));
    let error = inline_remote_document(&client, remote, &connection(settings(1024)))
        .await
        .expect_err("the download fails");
    let OcrError::Transport(OcrTransportError::Http { status, .. }) = error else {
        panic!("the failed download keeps its status");
    };
    assert_eq!(status, 500);
}

#[tokio::test]
async fn a_stalled_download_times_out() {
    let server = MockServer::start(vec![delayed(
        reply("200 OK", &[], b"abc"),
        Duration::from_millis(300),
    )])
    .await;
    let client = ocr_client(settings(1024), Duration::from_millis(50));
    let remote = document(&format!("{}/document", server.base));
    let error = inline_remote_document(&client, remote, &connection(settings(1024)))
        .await
        .expect_err("the stalled download times out");
    assert_eq!(error.http_status_code(), Some(408));
}
