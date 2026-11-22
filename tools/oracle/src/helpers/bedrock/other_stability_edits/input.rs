use std::collections::BTreeMap;

use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

// internally tagged, so each variant's payload struct sees only its own fields
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    ModelSelection(ModelSelectionCall),
    SupportedParams,
    Map(MapCall),
    Request(RequestCall),
    Response(ResponseCall),
    Multipart,
    Url,
    Environment(EnvironmentCall),
    ErrorClass(ErrorClassCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSelectionCall {
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) options: ImageParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) image: Option<ImageSpec>,
    #[serde(default)]
    pub(crate) options: ImageParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
}

// The wheel reads a file-like value before this code sees it, so an image arrives as its bytes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageSpec {
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentCall {
    #[serde(default)]
    pub(crate) headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorClassCall {
    pub(crate) message: String,
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
}
