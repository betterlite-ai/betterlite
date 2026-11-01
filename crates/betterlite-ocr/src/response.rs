use serde::{Deserialize, Serialize};

use crate::json::{Json, Object};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OCRPageDimensions {
    #[serde(default)]
    pub dpi: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default)]
    pub width: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct OCRPageImage {
    #[serde(default)]
    pub image_base64: Option<String>,
    #[serde(default)]
    pub bbox: Option<Object>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct OCRPage {
    pub index: i64,
    pub markdown: String,
    #[serde(default)]
    pub images: Option<Vec<OCRPageImage>>,
    #[serde(default)]
    pub dimensions: Option<OCRPageDimensions>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct OCRUsageInfo {
    #[serde(default)]
    pub pages_processed: Option<i64>,
    #[serde(default)]
    pub pages_processed_annotation: Option<i64>,
    #[serde(default)]
    pub credits: Option<f64>,
    #[serde(default)]
    pub doc_size_bytes: Option<i64>,
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct OCRResponse {
    pub pages: Vec<OCRPage>,
    pub model: String,
    #[serde(default)]
    pub document_annotation: Option<Json>,
    #[serde(default)]
    pub usage_info: Option<OCRUsageInfo>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tables: Option<Vec<Object>>,
    #[serde(default, rename = "keyValuePairs")]
    pub key_value_pairs: Option<Vec<Object>>,
    #[serde(default = "default_object")]
    pub object: String,
    #[serde(flatten)]
    pub extra: Object,
    // LiteLLM keeps this in OCRResponse's private _hidden_params, never on the wire
    #[serde(skip)]
    pub provider_native_response: Option<Json>,
}

fn default_object() -> String {
    "ocr".to_owned()
}

mod tests;
