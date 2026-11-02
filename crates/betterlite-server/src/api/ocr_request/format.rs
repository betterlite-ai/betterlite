use betterlite_ocr::json::{Json, Object};
use betterlite_ocr::request_format::{
    OCR_REQUEST_FORMAT_PARAM, RequestFormat, parse_ocr_request_format,
};

use super::error::OcrRequestError;

// An explicit req_format in the body wins over the x-req-format header.
pub(super) fn resolve(
    mut fields: Object,
    header_value: Option<&str>,
) -> Result<(Object, Option<RequestFormat>), OcrRequestError> {
    let body_value = fields
        .shift_remove(OCR_REQUEST_FORMAT_PARAM)
        .filter(|value| !matches!(value, Json::Null));
    let format_source =
        body_value.or_else(|| header_value.map(|header| Json::Text(header.to_owned())));
    let Some(format_value) = format_source else {
        return Ok((fields, None));
    };
    let request_format = parse_ocr_request_format(&format_value)?;
    Ok((fields, Some(request_format)))
}

mod tests;
