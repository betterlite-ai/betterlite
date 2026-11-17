use serde::Deserialize;

use betterlite_prompts::arguments::json::Object;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    TitanG1Request(G1Request),
    TitanV2DimensionsNormalize(V2DimensionsNormalize),
    TitanV2BinaryEmbedding(V2BinaryEmbedding),
    TitanMultimodalImage(MultimodalImage),
    CohereV3InputType(CohereV3InputType),
    TokenCountResponse(TokenCountResponse),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct G1Request {
    pub(crate) non_default: Object,
    pub(crate) optional_params: Object,
    pub(crate) input: String,
    pub(crate) inference_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct V2DimensionsNormalize {
    pub(crate) settings: Vec<SettingsStep>,
    pub(crate) non_default: Object,
    pub(crate) optional_params: Object,
    pub(crate) input: String,
    pub(crate) inference_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct V2BinaryEmbedding {
    pub(crate) model: String,
    pub(crate) response_list: Vec<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultimodalImage {
    pub(crate) non_default: Object,
    pub(crate) optional_params: Object,
    pub(crate) input: String,
    pub(crate) inference_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohereV3InputType {
    pub(crate) model: String,
    pub(crate) input: Vec<String>,
    pub(crate) optional_params: Object,
    pub(crate) non_default: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenCountResponse {
    pub(crate) provider: ResponseProvider,
    pub(crate) model: String,
    pub(crate) response_list: Vec<Object>,
    pub(crate) batch_data: Option<Vec<Object>>,
}

// one AmazonTitanV2Config(...) construction; a null value is the same as an absent one
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsStep {
    pub(crate) normalize: Option<bool>,
    pub(crate) dimensions: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseProvider {
    G1,
    V2,
    Multimodal,
}
