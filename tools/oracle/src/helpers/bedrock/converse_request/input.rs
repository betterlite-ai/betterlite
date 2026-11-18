use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) model_cost: ParamMap,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    #[serde(default)]
    pub(crate) litellm_params: Object,
    pub(crate) global_drop: bool,
    pub(crate) modify_params: bool,
    pub(crate) neutralize_orphaned_blocks: bool,
    #[serde(default)]
    pub(crate) request_metadata_fields: Option<Vec<String>>,
}
