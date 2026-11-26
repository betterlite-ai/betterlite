use std::collections::BTreeMap;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_openai::api::image_contracts::records::ImageParams;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model_cost: ParamMap,
    pub(crate) calls: Vec<Call>,
}

// internally tagged, so each variant's payload struct sees only its own fields
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    SupportedParams,
    Map(MapCall),
    Transform(TransformCall),
    Multipart,
    Url,
    Environment(EnvironmentCall),
    ErrorClass(ErrorClassCall),
    Eligibility(ModelsCall),
    Selection(ModelsCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    #[serde(default)]
    pub(crate) options: ImageParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformCall {
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) image: Option<FileSpec>,
    #[serde(default)]
    pub(crate) mask: Option<FileSpec>,
    #[serde(default)]
    pub(crate) options: ImageParams,
}

// The wheel reads a file-like value before this code sees it, so an image arrives as bytes.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum FileSpec {
    Bytes { data: String },
    Text { text: String },
    Tuple,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelsCall {
    pub(crate) models: Vec<String>,
}
