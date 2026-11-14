use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    VariationImagePart,
    #[serde(rename = "n_size_format_user")]
    NSizeFormatUser,
    OmittedDefaultFields,
    ProviderError,
    BinaryFileDescriptor,
    VariationMissingConfigData,
    VariationReplyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: Option<u16>,
    pub(crate) headers: ImageHeaders,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) model: Option<String>,
    pub(crate) image_filename: Option<String>,
    #[serde(default)]
    pub(crate) image_content: String,
    pub(crate) image_content_type: Option<String>,
    #[serde(default)]
    pub(crate) optional_params: ImageParams,
    #[serde(default)]
    pub(crate) non_default_params: ImageParams,
    #[serde(default)]
    pub(crate) existing_params: ImageParams,
    #[serde(default)]
    pub(crate) async_call: bool,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) timeout: Option<i64>,
    pub(crate) client: Option<Kind>,
    #[serde(default)]
    pub(crate) reply: Reply,
    pub(crate) failure: Option<Failure>,
    #[serde(default)]
    pub(crate) missing_config: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) probes: Vec<Probe>,
}
