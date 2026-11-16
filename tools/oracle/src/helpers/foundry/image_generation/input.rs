use betterlite_foundry::image_generation::params::ImageParams;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) models: Option<Vec<String>>,
    pub(super) flux2: Option<Vec<String>>,
    pub(super) paths: Option<Vec<String>>,
    pub(super) urls: Option<Vec<UrlCall>>,
    pub(super) supported: Option<Vec<String>>,
    pub(super) params: Option<Vec<ParamsCall>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UrlCall {
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ParamsCall {
    pub(super) model: String,
    pub(super) non_default_params: ImageParams,
    pub(super) optional_params: ImageParams,
    pub(super) drop_params: bool,
}
