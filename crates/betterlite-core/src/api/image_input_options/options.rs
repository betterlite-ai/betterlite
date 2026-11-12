#![expect(
    clippy::derive_partial_eq_without_eq,
    clippy::disallowed_types,
    reason = "imageConfig is LiteLLM's dict[str, Any] and provider capability extensions are open by design; the JSON values they carry have no Eq"
)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    Transparent,
    Opaque,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFidelity {
    High,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    High,
    Medium,
    Low,
    Standard,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    Url,
    #[serde(rename = "b64_json")]
    B64Json,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageEditOptionalRequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_fidelity: Option<InputFidelity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,
    #[serde(rename = "n", skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(rename = "imageConfig", skip_serializing_if = "Option::is_none")]
    pub image_config: Option<BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub extensions: IndexMap<String, Value>,
}

impl ImageEditOptionalRequestParams {
    pub(super) fn present_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.background.is_some() {
            names.push("background".to_owned());
        }
        if self.input_fidelity.is_some() {
            names.push("input_fidelity".to_owned());
        }
        if self.mask.is_some() {
            names.push("mask".to_owned());
        }
        if self.count.is_some() {
            names.push("n".to_owned());
        }
        if self.quality.is_some() {
            names.push("quality".to_owned());
        }
        if self.response_format.is_some() {
            names.push("response_format".to_owned());
        }
        if self.size.is_some() {
            names.push("size".to_owned());
        }
        if self.user.is_some() {
            names.push("user".to_owned());
        }
        if self.image_config.is_some() {
            names.push("imageConfig".to_owned());
        }
        names.extend(self.extensions.keys().cloned());
        names
    }

    pub(super) fn remove(&mut self, name: &str) {
        match name {
            "background" => self.background = None,
            "input_fidelity" => self.input_fidelity = None,
            "mask" => self.mask = None,
            "n" => self.count = None,
            "quality" => self.quality = None,
            "response_format" => self.response_format = None,
            "size" => self.size = None,
            "user" => self.user = None,
            "imageConfig" => self.image_config = None,
            _ => {
                self.extensions.shift_remove(name);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEditImage {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageEditRequestParams {
    pub image: Option<ImageEditImage>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub options: ImageEditOptionalRequestParams,
}
