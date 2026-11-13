use serde::Deserialize;

use crate::ocr_error::OcrError;

const UNSUPPORTED_DOCUMENT: &str = "UnsupportedDocumentException";
const UNSUPPORTED_DOCUMENT_HINT: &str = "the aws_textract provider uses Textract's synchronous API, which accepts a JPEG, PNG or a single-page PDF or TIFF; other formats and multi-page documents are not supported";

#[derive(Deserialize)]
pub(crate) struct AwsError {
    #[serde(rename = "__type", default)]
    kind: String,
    #[serde(rename = "Message", alias = "message", default)]
    message: String,
}

// Textract answers both an unsupported format and a multi-page PDF or TIFF with a bare "unsupported document format"
pub(crate) fn textract_error(
    body: String,
    status: u16,
    headers: Vec<(String, String)>,
) -> OcrError {
    let unsupported = serde_json::from_str::<AwsError>(&body)
        .ok()
        .filter(|error| error.kind.ends_with(UNSUPPORTED_DOCUMENT));
    OcrError::Provider {
        status,
        body: match unsupported {
            Some(error) => {
                format!(
                    "{UNSUPPORTED_DOCUMENT}: {}. {UNSUPPORTED_DOCUMENT_HINT}",
                    error.message
                )
            }
            None => body,
        },
        headers,
    }
}
