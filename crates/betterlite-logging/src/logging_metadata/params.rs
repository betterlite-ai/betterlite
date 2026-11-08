use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type MetadataMap = IndexMap<String, Json>;

// The litellm_params keys the logging metadata helpers read; other keys never reach them
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LitellmParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_metadata: Option<MetadataMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub litellm_trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_variables: Option<Json>,
    // the slot scrub_sensitive_keys_in_metadata moves a masking callable into
    #[serde(
        default,
        rename = "_langfuse_masking_function",
        skip_serializing_if = "Option::is_none"
    )]
    pub langfuse_masking_function: Option<Json>,
}
