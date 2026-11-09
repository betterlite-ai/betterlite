use rstest::rstest;

use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_http::ocr;
use crate::ocr_http::tests::support::{MockServer, reply};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::textract_ocr::detect::TextractDetectTextConfig;

use super::support::{NoHooks, json, region_params, request, signing_client};

const MODEL: &str = "detect-document-text";

fn detect_response(raw: &str) -> OCRResponse {
    TextractDetectTextConfig
        .transform_ocr_response(MODEL, raw.as_bytes(), RequestFormat::Litellm)
        .expect("the Textract answer decodes")
}

#[rstest]
#[case::one_page_without_page_numbers(
    r#"{"DetectDocumentTextModelVersion":"1.0","DocumentMetadata":{"Pages":1},"Blocks":[
        {"BlockType":"PAGE"},
        {"BlockType":"LINE","Text":"Invoice 12345"},
        {"BlockType":"WORD","Text":"Invoice"},
        {"BlockType":"WORD","Text":"12345"},
        {"BlockType":"LINE","Text":"total 67.89"}]}"#,
    vec![(0, "Invoice 12345\ntotal 67.89")],
    Some(1)
)]
#[case::pages_out_of_order(
    r#"{"DocumentMetadata":{"Pages":2},"Blocks":[
        {"BlockType":"LINE","Text":"second","Page":2},
        {"BlockType":"LINE","Text":"first","Page":1},
        {"BlockType":"LINE","Text":"also second","Page":2}]}"#,
    vec![(0, "first"), (1, "second\nalso second")],
    Some(2)
)]
#[case::missing_metadata_counts_the_pages_with_text(
    r#"{"Blocks":[{"BlockType":"LINE","Text":"only"}]}"#,
    vec![(0, "only")],
    Some(1)
)]
#[case::the_document_page_count_wins_over_the_pages_with_text(
    r#"{"DocumentMetadata":{"Pages":3},"Blocks":[
        {"BlockType":"LINE","Text":"first","Page":1},
        {"BlockType":"LINE","Text":"third","Page":3}]}"#,
    vec![(0, "first"), (2, "third")],
    Some(3)
)]
#[case::a_page_without_lines_is_dropped(
    r#"{"Blocks":[
        {"BlockType":"PAGE","Page":1},
        {"BlockType":"LINE","Text":"only","Page":2}]}"#,
    vec![(1, "only")],
    Some(1)
)]
#[case::a_line_without_a_page_belongs_to_page_one(
    r#"{"Blocks":[
        {"BlockType":"LINE","Text":"explicit","Page":1},
        {"BlockType":"LINE","Text":"implicit"}]}"#,
    vec![(0, "explicit\nimplicit")],
    Some(1)
)]
#[case::a_null_page_belongs_to_page_one(
    r#"{"Blocks":[
        {"BlockType":"LINE","Text":"explicit","Page":1},
        {"BlockType":"LINE","Text":"null","Page":null}]}"#,
    vec![(0, "explicit\nnull")],
    Some(1)
)]
#[case::metadata_without_pages_counts_the_pages_with_text(
    r#"{"DocumentMetadata":{},"Blocks":[{"BlockType":"LINE","Text":"only"}]}"#,
    vec![(0, "only")],
    Some(1)
)]
#[case::blank_document(
    r#"{"DocumentMetadata":{"Pages":1}}"#,
    vec![],
    Some(1)
)]
fn response_lines_become_one_markdown_page_per_document_page(
    #[case] raw: &str,
    #[case] expected_pages: Vec<(i64, &str)>,
    #[case] expected_pages_processed: Option<i64>,
) {
    let response = detect_response(raw);

    let pages: Vec<(i64, &str)> = response
        .pages
        .iter()
        .map(|page| (page.index, page.markdown.as_str()))
        .collect();
    assert_eq!(pages, expected_pages);
    assert_eq!(response.model, MODEL);
    assert_eq!(
        response.usage_info.unwrap().pages_processed,
        expected_pages_processed
    );
}

#[test]
fn native_responses_keep_the_textract_body() {
    let raw = r#"{"DocumentMetadata":{"Pages":1},"Blocks":[{"BlockType":"LINE","Text":"only"}]}"#;

    let response = TextractDetectTextConfig
        .transform_ocr_response(MODEL, raw.as_bytes(), RequestFormat::Native)
        .expect("the native answer decodes");

    assert_eq!(response.provider_native_response, Some(json(raw)));
    assert_eq!(response.pages.len(), 1);
}

#[tokio::test]
async fn a_call_sends_the_signed_request_and_returns_the_pages() {
    let raw = r#"{"DocumentMetadata":{"Pages":1},"Blocks":[{"BlockType":"LINE","Text":"Invoice 12345"}]}"#;
    let server = MockServer::start(vec![reply(
        "200 OK",
        &[("content-type", "application/x-amz-json-1.1")],
        raw.as_bytes(),
    )])
    .await;
    let (_log, client) = signing_client();
    let request = request(
        "detect-document-text",
        Some(server.base.as_str()),
        region_params("us-east-1"),
    );

    let response = ocr(&TextractDetectTextConfig, &client, &request, &NoHooks)
        .await
        .expect("the call returns the pages");

    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].markdown, "Invoice 12345");
    assert_eq!(response.model, MODEL);

    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert_signed_detect_request(&requests[0]);
}

fn assert_signed_detect_request(request: &str) {
    let lowered = request.to_ascii_lowercase();
    assert!(lowered.starts_with("post "), "{request}");
    assert!(
        lowered.contains("x-amz-target: textract.detectdocumenttext"),
        "{request}"
    );
    assert!(
        lowered.contains("content-type: application/x-amz-json-1.1"),
        "{request}"
    );
    assert!(
        lowered.contains("authorization: aws4-hmac-sha256 recorded"),
        "{request}"
    );
    assert!(
        request.contains(r#"{"Document":{"Bytes":"aGVsbG8="}}"#),
        "{request}"
    );
}
