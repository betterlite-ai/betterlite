use async_trait::async_trait;
use thiserror::Error;

pub const DEFAULT_OCR_MODEL: &str = "mistral/mistral-ocr-latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrDocumentKind {
    ImageUrl,
    DocumentUrl,
}

impl OcrDocumentKind {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::ImageUrl => "image_url",
            Self::DocumentUrl => "document_url",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrDocument {
    pub kind: OcrDocumentKind,
    pub data_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrRequest {
    pub model: String,
    pub document: OcrDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPage {
    pub markdown: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OcrResponse {
    pub pages: Vec<OcrPage>,
}

impl OcrResponse {
    #[must_use]
    pub fn from_pages(pages: Vec<String>) -> Self {
        Self {
            pages: pages
                .into_iter()
                .map(|markdown| OcrPage { markdown })
                .collect(),
        }
    }
}

#[async_trait]
pub trait OcrOperation: Send + Sync {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OcrError {
    #[error("{message}")]
    Failed { message: String },
}

#[must_use]
pub(crate) fn joined_markdown(response: &OcrResponse) -> Option<String> {
    if response.pages.is_empty() {
        return None;
    }
    let markdown: Vec<&str> = response
        .pages
        .iter()
        .map(|page| page.markdown.as_str())
        .collect();
    Some(markdown.join("\n\n"))
}
