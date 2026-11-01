use std::string::FromUtf8Error;

use betterlite_ocr::request_format::RequestFormatError;

#[derive(Debug, thiserror::Error)]
pub enum OcrRequestError {
    #[error("failed to parse multipart form data")]
    MultipartParse(#[source] multipart_rs::MultipartError),
    #[error("OCR request body must be a JSON object")]
    InvalidJson(#[source] serde_json::Error),
    #[error("OCR request body is not a JSON object")]
    NotAJsonObject,
    #[error("document type 'file' is not supported through the JSON API")]
    FileTypeNotSupported,
    #[error("reducto:// file IDs are not accepted through the proxy OCR API")]
    ReductoIdRejected,
    #[error("multipart OCR request must include a 'file' field with the document to process")]
    MissingFile,
    #[error("uploaded file is empty")]
    EmptyFile,
    #[error("OCR file exceeds the size limit")]
    FileTooLarge,
    #[error("empty request body")]
    EmptyBody,
    #[error("multipart text field is not valid UTF-8")]
    InvalidTextEncoding(#[source] FromUtf8Error),
    #[error(transparent)]
    InvalidRequestFormat(#[from] RequestFormatError),
}

mod tests;
