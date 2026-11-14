use std::fmt;
use std::sync::Arc;

use betterlite_models::identity::provider_id::ProviderId;

use crate::aws_settings::credential_cache::CredentialCache;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::BedrockMantle;

// Mantle's Responses API offers neither native file search nor a websocket surface, whatever the model.
pub const SUPPORTS_NATIVE_FILE_SEARCH: bool = false;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

// gpt frontier models are served under /openai/v1, every other Responses model under /v1.
pub const DEFAULT_USE_OPENAI_PATH: bool = true;

// B625 signs with the cache the caller injects; a config without one leaves signing to the caller.
pub struct MantleResponsesConfig {
    pub credential_cache: Option<Arc<CredentialCache>>,
    pub use_openai_path: bool,
}

impl Default for MantleResponsesConfig {
    fn default() -> Self {
        Self {
            credential_cache: None,
            use_openai_path: DEFAULT_USE_OPENAI_PATH,
        }
    }
}

impl fmt::Debug for MantleResponsesConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MantleResponsesConfig")
            .field("use_openai_path", &self.use_openai_path)
            .finish_non_exhaustive()
    }
}
