pub mod analyze;
pub mod detect;
mod document;
pub mod environment;
mod error;
mod markdown;
mod response;
mod tests;
pub mod wire;

use crate::ocr_error::OcrError;

const PROVIDER: &str = "aws_textract";
const SUPPORTED_MODELS: [&str; 2] = ["detect-document-text", "analyze-document"];

// Textract has operations rather than models; the model slot of aws_textract/<model> names the one to call
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextractOperation {
    DetectDocumentText,
    AnalyzeDocument,
}

impl TextractOperation {
    pub fn from_model(model: &str) -> Result<Self, OcrError> {
        match model.to_ascii_lowercase().as_str() {
            "detect-document-text" => Ok(Self::DetectDocumentText),
            "analyze-document" => Ok(Self::AnalyzeDocument),
            _ => Err(OcrError::InvalidModel {
                provider: PROVIDER,
                model: model.to_owned(),
                supported: &SUPPORTED_MODELS,
            }),
        }
    }

    pub(crate) const fn target(self) -> &'static str {
        match self {
            Self::DetectDocumentText => "Textract.DetectDocumentText",
            Self::AnalyzeDocument => "Textract.AnalyzeDocument",
        }
    }
}
