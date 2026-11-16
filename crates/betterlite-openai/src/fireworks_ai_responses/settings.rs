use serde::Deserialize;

use crate::fireworks_ai_common::{FireworksEnv, SessionMetadata};

#[derive(Debug, Clone, Default)]
pub struct FireworksResponsesEnv {
    pub keys: FireworksEnv,
    pub api_base: Option<String>,
}

// LiteLLM's _session_params reads litellm_session_id and litellm_metadata from the request's extra fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SessionSource {
    #[serde(default)]
    pub litellm_session_id: Option<String>,
    #[serde(default)]
    pub litellm_metadata: Option<SessionMetadata>,
}
