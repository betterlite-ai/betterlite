#![cfg(test)]

use rstest::rstest;

use betterlite_ocr::json::Json;
use betterlite_ocr::request_format::RequestFormatError;

use super::OcrRequestError;

#[rstest]
#[case::not_a_json_object(
    OcrRequestError::NotAJsonObject,
    "OCR request body is not a JSON object"
)]
#[case::file_type_not_supported(
    OcrRequestError::FileTypeNotSupported,
    "document type 'file' is not supported through the JSON API"
)]
#[case::reducto_id_rejected(
    OcrRequestError::ReductoIdRejected,
    "reducto:// file IDs are not accepted through the proxy OCR API"
)]
#[case::missing_file(
    OcrRequestError::MissingFile,
    "multipart OCR request must include a 'file' field with the document to process"
)]
#[case::empty_file(OcrRequestError::EmptyFile, "uploaded file is empty")]
#[case::file_too_large(OcrRequestError::FileTooLarge, "OCR file exceeds the size limit")]
#[case::empty_body(OcrRequestError::EmptyBody, "empty request body")]
fn error_text_is_betterlites_own(#[case] error: OcrRequestError, #[case] message: &str) {
    assert_eq!(error.to_string(), message);
}

#[test]
fn invalid_request_format_is_transparent_to_its_source() {
    let error = OcrRequestError::from(RequestFormatError::Invalid {
        value: "yaml".to_owned(),
    });
    assert_eq!(
        error.to_string(),
        "invalid req_format yaml, expected \"litellm\" or \"native\""
    );
}

#[test]
fn invalid_text_encoding_is_betterlites_own() {
    let source = String::from_utf8(vec![0xFF]).unwrap_err();
    let error = OcrRequestError::InvalidTextEncoding(source);
    assert_eq!(error.to_string(), "multipart text field is not valid UTF-8");
}

#[test]
fn invalid_json_is_betterlites_own() {
    let source = serde_json::from_str::<Json>("{").unwrap_err();
    let error = OcrRequestError::InvalidJson(source);
    assert_eq!(error.to_string(), "OCR request body must be a JSON object");
}

#[test]
fn multipart_parse_is_betterlites_own() {
    let error = OcrRequestError::MultipartParse(multipart_rs::MultipartError::InvalidBoundary);
    assert_eq!(error.to_string(), "failed to parse multipart form data");
}
