use rstest::rstest;

use crate::ocr_config::{OcrProviderConfig as _, OcrTransform};
use crate::ocr_error::OcrError;
use crate::ocr_http::tests::support::{MockServer, reply};
use crate::textract_ocr::detect::{DetectDocumentTextRequest, TextractDetectTextConfig};
use crate::textract_ocr::document::document_bytes;

use super::support::{NoHooks, body_json, document, json, region_params, request, signing_client};

const TEN_MEBIBYTES: usize = 10 * 1024 * 1024;

fn sync_transform(source: &str) -> Result<DetectDocumentTextRequest, OcrError> {
    TextractDetectTextConfig.transform_ocr_request(
        document(source),
        OcrTransform {
            model: "detect-document-text",
            optional_params: &(),
            headers: &[],
        },
    )
}

#[rstest]
#[case::image_url("data:image/png;base64,aGVsbG8=", "aGVsbG8=")]
#[case::document_url("data:application/pdf;base64,YWJj", "YWJj")]
#[case::percent_encoded_data_uri_is_re_encoded_as_base64("data:,abc", "YWJj")]
fn document_bytes_are_the_base64_payload_without_the_data_uri_envelope(
    #[case] source: &str,
    #[case] expected: &str,
) {
    assert_eq!(document_bytes(&document(source)).unwrap().bytes, expected);
}

#[rstest]
#[case::remote_url("https://example.com/a.pdf".to_owned())]
#[case::invalid_base64("data:image/png;base64,@@@".to_owned())]
fn document_bytes_refuse_what_the_sync_api_cannot_take(#[case] source: String) {
    let error = sync_transform(&source).unwrap_err();

    assert!(matches!(&error, OcrError::InvalidDataUri), "{error}");
}

#[test]
fn a_document_at_the_sync_limit_is_accepted() {
    let source = format!("data:,{}", "a".repeat(TEN_MEBIBYTES));

    let encoded = document_bytes(&document(&source)).expect("ten mebibytes fit the sync cap");

    assert!(!encoded.bytes.is_empty());
}

#[test]
fn a_document_one_byte_over_the_sync_limit_is_refused() {
    let source = format!("data:,{}", "a".repeat(TEN_MEBIBYTES + 1));

    let error = sync_transform(&source).unwrap_err();

    assert!(
        matches!(&error, OcrError::InlineDocumentTooLarge),
        "{error}"
    );
}

#[tokio::test]
async fn a_document_over_the_sync_limit_fails_before_signing() {
    let (log, client) = signing_client();
    let mut request = request("detect-document-text", None, region_params("us-east-1"));
    request.document = document(&format!("data:,{}", "a".repeat(TEN_MEBIBYTES + 1)));

    let error = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .unwrap_err();

    assert!(
        matches!(&error, OcrError::InlineDocumentTooLarge),
        "{error}"
    );
    assert!(log.calls().is_empty());
}

#[tokio::test]
async fn a_non_inline_document_fails_before_any_request() {
    let (log, client) = signing_client();
    let mut request = request("detect-document-text", None, region_params("us-east-1"));
    request.document = document("not-a-data-uri");

    let error = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .unwrap_err();

    assert!(matches!(&error, OcrError::InvalidDataUri), "{error}");
    assert!(log.calls().is_empty());
}

#[tokio::test]
async fn a_remote_document_is_downloaded_and_inlined_before_signing() {
    let server = MockServer::start(vec![reply(
        "200 OK",
        &[("content-type", "image/png")],
        b"abc",
    )])
    .await;
    let (_log, client) = signing_client();
    let mut request = request(
        "detect-document-text",
        Some(server.base.as_str()),
        region_params("us-east-1"),
    );
    request.document = document(&format!("{}/image", server.base));

    let outbound = TextractDetectTextConfig
        .prepare_request(&request, &client, &NoHooks)
        .await
        .expect("the downloaded image becomes the request body");

    assert_eq!(
        body_json(&outbound),
        json(r#"{"Document":{"Bytes":"YWJj"}}"#)
    );
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    let lowered = requests[0].to_ascii_lowercase();
    assert!(lowered.starts_with("get /image "), "{}", requests[0]);
    assert!(!lowered.contains("authorization:"), "{}", requests[0]);
}
