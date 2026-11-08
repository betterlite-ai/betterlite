use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type MetadataMap = IndexMap<String, Json>;

// The litellm_params keys the logging metadata helpers read; other keys never reach them
#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LitellmParams {
    pub metadata: Option<MetadataMap>,
    pub litellm_metadata: Option<MetadataMap>,
    pub litellm_session_id: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub prompt_id: Option<Json>,
    pub prompt_variables: Option<Json>,
    // the slot scrub_sensitive_keys_in_metadata moves a masking callable into
    #[serde(rename = "_langfuse_masking_function")]
    pub langfuse_masking_function: Option<Json>,
}
