use std::collections::BTreeMap;

use betterlite_bedrock::event_frames::beta::BetaHeader;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::Value;

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
pub(crate) struct CaseFlag(pub(crate) bool);

// The case lists the metadata members as a JSON array, whose order survives any object key sorting.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataPair {
    pub(crate) key: String,
    pub(crate) value: Value,
}

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
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, BetaHeader>,
    pub(crate) global_drop: CaseFlag,
    pub(crate) modify_params: CaseFlag,
    pub(crate) neutralize_orphaned_blocks: CaseFlag,
    #[serde(default)]
    pub(crate) request_metadata_fields: Option<Vec<String>>,
    // A requestMetadata whose members arrive in caller order, which the case carries as pairs.
    #[serde(default)]
    pub(crate) request_metadata_order: Vec<MetadataPair>,
}
