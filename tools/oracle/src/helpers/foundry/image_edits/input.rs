use std::collections::BTreeMap;

use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) models: Option<Vec<String>>,
    pub(super) supported: Option<Vec<String>>,
    pub(super) auths: Option<Vec<AuthStep>>,
    pub(super) urls: Option<Vec<UrlStep>>,
    pub(super) transforms: Option<Vec<TransformStep>>,
    pub(super) replies: Option<Vec<ReplyStep>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthStep {
    pub(super) model: String,
    pub(super) api_key: Option<String>,
    pub(super) global_api_key: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) params: Option<ImageParams>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlStep {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) global_api_version: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransformStep {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: Option<ImageField>,
    pub(super) images: Option<Vec<Option<ImageField>>>,
    #[serde(default)]
    pub(super) options: ImageParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageField {
    pub(super) kind: ImageKind,
    #[serde(default)]
    pub(super) text: String,
    #[serde(default)]
    pub(super) position: usize,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ImageKind {
    Bytes,
    Buffered,
    Reader,
    Unsupported,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyStep {
    pub(super) body: String,
    #[serde(default = "default_status")]
    pub(super) status: u16,
}

const fn default_status() -> u16 {
    200
}
