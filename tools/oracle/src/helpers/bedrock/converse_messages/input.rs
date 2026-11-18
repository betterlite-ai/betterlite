use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Guarded(GuardedInput),
    HandleOrphaned(HandleOrphanedInput),
    TransformSystem(TransformSystemInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardedInput {
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandleOrphanedInput {
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) optional_params: ParamMap,
    pub(crate) neutralize_orphaned_blocks: bool,
    pub(crate) modify_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformSystemInput {
    pub(crate) messages: Vec<Message>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) model_cost: ParamMap,
}
