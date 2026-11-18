use std::sync::Arc;

use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redact::secret::Redactor;

use ports::{
    ChunkDeploymentHook, ResponseMetadataTarget, ResponsesStreamCacheHandler,
    ResponsesStreamCacheTarget, ResponsesStreamFailureTarget, ResponsesStreamSuccessTarget,
    SuccessDeploymentHook,
};

pub mod cache;
pub mod deployment;
pub mod events;
pub mod failed;
pub mod failure;
pub mod logging;
pub mod ports;
pub mod post_success;
pub mod state;

mod tests;

// The injected dependencies of the Responses stream hook methods: LiteLLM reaches each through
// logging_obj, litellm.cache or the callback list, and none of them may be rebuilt here.
pub struct ResponsesStreamHooks<'context> {
    pub cache: Option<Arc<dyn ResponsesStreamCacheTarget>>,
    pub cache_handler: Option<Arc<dyn ResponsesStreamCacheHandler>>,
    pub success: Option<Arc<dyn ResponsesStreamSuccessTarget>>,
    pub failure: Option<Arc<dyn ResponsesStreamFailureTarget>>,
    pub metadata: Option<Arc<dyn ResponseMetadataTarget>>,
    pub success_hook: Option<Arc<dyn SuccessDeploymentHook>>,
    pub chunk_hooks: Vec<Arc<dyn ChunkDeploymentHook>>,
    pub clock: Arc<dyn SuccessClock>,
    pub redactor: &'context Redactor,
}
