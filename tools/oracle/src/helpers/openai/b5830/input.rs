use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::openai_base_config::Headers;
use serde::Deserialize;
use std::collections::BTreeMap;
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GenerationParams { cases: Vec<GenerationParamsCase> },
    GenerationRequest { cases: Vec<RequestCase> },
    GenerationReply { cases: Vec<ReplyCase> },
    EditParams { cases: Vec<ImageParams> },
    EditRequest { cases: Vec<RequestCase> },
    EditReply { cases: Vec<ReplyCase> },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GenerationParamsCase {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) non_default_params: ImageParams,
    #[serde(default)]
    pub(crate) optional_params: ImageParams,
    #[serde(default)]
    pub(crate) drop_params: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCase {
    pub(crate) model: String,
    pub(crate) prompt: Option<String>,
    pub(crate) image: Option<ImageCase>,
    #[serde(default)]
    pub(crate) optional_params: ImageParams,
    #[serde(default)]
    pub(crate) headers: Headers,
    pub(crate) api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_base: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ImageCase {
    Bytes { data: String },
    Reader { data: String, position: u64 },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    pub(crate) model: String,
    pub(crate) body: String,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Headers,
    #[serde(default)]
    pub(crate) created: i64,
}
