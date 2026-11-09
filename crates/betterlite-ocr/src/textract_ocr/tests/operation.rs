use rstest::rstest;

use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_error::OcrError;
use crate::textract_ocr::TextractOperation;
use crate::textract_ocr::analyze::TextractAnalyzeDocumentConfig;
use crate::textract_ocr::detect::TextractDetectTextConfig;
use crate::textract_ocr::document::document_bytes;

const HEALTH_CHECK_IMAGE_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC";

const AWS_CREDENTIAL_NAMES: [&str; 11] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION_NAME",
    "AWS_REGION",
    "AWS_SESSION_NAME",
    "AWS_PROFILE_NAME",
    "AWS_ROLE_NAME",
    "AWS_WEB_IDENTITY_TOKEN",
    "AWS_STS_ENDPOINT",
    "AWS_EXTERNAL_ID",
];

#[rstest]
#[case::detect("detect-document-text", TextractOperation::DetectDocumentText)]
#[case::analyze("analyze-document", TextractOperation::AnalyzeDocument)]
#[case::any_case("Analyze-Document", TextractOperation::AnalyzeDocument)]
fn a_model_names_its_operation(#[case] model: &str, #[case] expected: TextractOperation) {
    assert_eq!(TextractOperation::from_model(model).unwrap(), expected);
}

#[rstest]
#[case::misspelled("analyse-document")]
#[case::operation_name_from_the_api("AnalyzeDocument")]
#[case::operation_litellm_does_not_call("analyze-expense")]
#[case::empty("")]
fn a_model_outside_the_operations_is_refused_with_the_supported_names(#[case] model: &str) {
    let error = TextractOperation::from_model(model).unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "invalid model: aws_textract has no model {model:?} - use one of: detect-document-text, analyze-document"
        )
    );
    assert_eq!(error.http_status_code(), Some(400));
    assert!(matches!(error, OcrError::InvalidModel { .. }));
}

#[test]
fn both_operations_ask_for_the_aws_credentials() {
    assert_eq!(
        TextractDetectTextConfig.secret_names(),
        AWS_CREDENTIAL_NAMES
    );
    assert_eq!(
        TextractAnalyzeDocumentConfig.secret_names(),
        AWS_CREDENTIAL_NAMES
    );
}

#[test]
fn the_health_check_document_is_the_one_by_one_png() {
    let detect = TextractDetectTextConfig.get_health_check_document();
    let analyze = TextractAnalyzeDocumentConfig.get_health_check_document();

    assert_eq!(detect.source(), HEALTH_CHECK_IMAGE_DATA_URI);
    assert_eq!(analyze.source(), HEALTH_CHECK_IMAGE_DATA_URI);
    assert_eq!(
        document_bytes(&detect)
            .expect("the health check image is inline")
            .bytes,
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC"
    );
}
