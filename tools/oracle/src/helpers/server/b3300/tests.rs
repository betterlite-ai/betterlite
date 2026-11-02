#![cfg(test)]
use betterlite_ocr::request_format::RequestFormatError;
use betterlite_server::api::ocr_request::error::OcrRequestError;

use super::parse_error;

#[test]
fn an_invalid_request_format_carries_litellms_400() {
    let error = OcrRequestError::InvalidRequestFormat(RequestFormatError::Invalid {
        value: "yaml".to_owned(),
    });
    let helper_error = parse_error(&error);
    assert_eq!(helper_error.status, Some(400));
}

#[test]
fn every_other_variant_carries_no_status() {
    let error = OcrRequestError::MissingFile;
    let helper_error = parse_error(&error);
    assert_eq!(helper_error.status, None);
}
