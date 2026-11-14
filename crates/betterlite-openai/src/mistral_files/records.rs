use serde::Deserialize;
fn default_purpose() -> String {
    String::from("batch")
}
fn default_deleted() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MistralFile {
    pub id: String,
    #[serde(default)]
    pub bytes: i64,
    pub created_at: Option<i64>,
    #[serde(default)]
    pub filename: String,
    #[serde(default = "default_purpose")]
    pub purpose: String,
    pub expires_at: Option<i64>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MistralFileList {
    #[serde(default)]
    pub data: Vec<MistralFile>,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MistralFileDeleted {
    pub id: String,
    #[serde(default = "default_deleted")]
    pub deleted: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MistralPurpose {
    FineTune,
    Batch,
    Ocr,
}
impl MistralPurpose {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::FineTune => "fine-tune",
            Self::Batch => "batch",
            Self::Ocr => "ocr",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUpload {
    pub url: String,
    pub filename: String,
    pub content_type: String,
    pub content: Vec<u8>,
    pub purpose: MistralPurpose,
}
