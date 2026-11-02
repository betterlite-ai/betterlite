use crate::json::Json;

pub const OCR_REQUEST_FORMAT_PARAM: &str = "req_format";
pub const OCR_REQUEST_FORMAT_HEADER: &str = "x-req-format";
pub const PROVIDER_NATIVE_RESPONSE_KEY: &str = "provider_native_response";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestFormat {
    Litellm,
    Native,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestFormatError {
    #[error("invalid req_format {value}, expected \"litellm\" or \"native\"")]
    Invalid { value: String },
}

pub fn parse_ocr_request_format(value: &Json) -> Result<RequestFormat, RequestFormatError> {
    let Json::Text(text) = value else {
        return Err(RequestFormatError::Invalid {
            value: "a non-string value".to_owned(),
        });
    };
    match text.trim().to_lowercase().as_str() {
        "litellm" => Ok(RequestFormat::Litellm),
        "native" => Ok(RequestFormat::Native),
        _ => Err(RequestFormatError::Invalid {
            value: text.clone(),
        }),
    }
}

mod tests;
