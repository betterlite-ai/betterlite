use rstest::rstest;

use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_error::OcrError;
use crate::ocr_http::ocr;
use crate::ocr_http::tests::support::{MockServer, reply};
use crate::textract_ocr::analyze::TextractAnalyzeDocumentConfig;
use crate::textract_ocr::detect::TextractDetectTextConfig;

use super::support::{NoHooks, region_params, request, signing_client};

const HINT: &str = "other formats and multi-page documents are not supported";

#[rstest]
#[case::bare_type(
    r#"{"__type":"UnsupportedDocumentException","Message":"Request has unsupported document format"}"#,
    Some("Request has unsupported document format")
)]
#[case::namespaced_type(
    r#"{"__type":"com.amazonaws.textract#UnsupportedDocumentException","Message":"bad"}"#,
    Some("bad")
)]
#[case::lowercase_message(
    r#"{"__type":"UnsupportedDocumentException","message":"bad"}"#,
    Some("bad")
)]
#[case::other_exception(r#"{"__type":"AccessDeniedException","Message":"no"}"#, None)]
#[case::json_without_a_type(r#"{"Message":"no"}"#, None)]
#[case::not_json("<html>bad gateway</html>", None)]
fn only_an_unsupported_document_gains_the_sync_api_hint(
    #[case] body: &str,
    #[case] hinted_message: Option<&str>,
) {
    let headers = vec![("x-amzn-requestid".to_owned(), "abc".to_owned())];

    let error = TextractDetectTextConfig.get_error_class(body.to_owned(), 400, headers.clone());

    let OcrError::Provider {
        status,
        body: reported,
        headers: returned,
    } = error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(status, 400);
    assert_eq!(returned, headers);
    match hinted_message {
        Some(message) => {
            assert!(reported.contains(message), "{reported}");
            assert!(reported.contains(HINT), "{reported}");
        }
        None => assert_eq!(reported, body),
    }
}

#[test]
fn an_unsupported_document_is_an_invalid_request_error() {
    let error = TextractAnalyzeDocumentConfig.get_error_class(
        r#"{"__type":"UnsupportedDocumentException","Message":"bad"}"#.to_owned(),
        400,
        Vec::new(),
    );

    let fields = error.fields();
    assert_eq!(fields.status, 400);
    assert_eq!(fields.error_type, "invalid_request_error");
    assert_eq!(fields.code, "400");
    assert_eq!(error.http_status_code(), Some(400));
}

#[tokio::test]
async fn a_non_success_answer_becomes_a_provider_error() {
    let body = r#"{"__type":"com.amazonaws.textract#UnsupportedDocumentException","Message":"Request has unsupported document format"}"#;
    let server = MockServer::start(vec![reply(
        "400 Bad Request",
        &[("x-amzn-requestid", "abc")],
        body.as_bytes(),
    )])
    .await;
    let (_log, client) = signing_client();
    let request = request(
        "detect-document-text",
        Some(server.base.as_str()),
        region_params("us-east-1"),
    );

    let error = ocr(&TextractDetectTextConfig, &client, &request, &NoHooks)
        .await
        .unwrap_err();

    assert_eq!(error.fields().status, 400);
    assert_eq!(error.fields().error_type, "invalid_request_error");
    let OcrError::Provider {
        status,
        body: reported,
        headers,
    } = error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(status, 400);
    assert!(
        reported.contains("Request has unsupported document format"),
        "{reported}"
    );
    assert!(reported.contains(HINT), "{reported}");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-amzn-requestid" && value == "abc"),
        "{headers:?}"
    );
}
