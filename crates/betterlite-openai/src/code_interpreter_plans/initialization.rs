use std::sync::Arc;

use betterlite_models::catalog::ModelCatalog;

use crate::sandbox_lifecycle::cache_operations::SandboxCache;

// LiteLLM's CodeInterpreterInterceptionLogger holds the deployment's settings, the model catalog provider resolution reads and the sandbox cache a call shares with its follow-ups
pub struct CodeInterpreterInterceptionLogger<C> {
    pub enabled: bool,
    pub enabled_providers: Option<Vec<String>>,
    pub sandbox_tool_name: Option<String>,
    pub sandbox_config: Option<C>,
    pub catalog: Arc<ModelCatalog>,
    pub cache: SandboxCache,
}

impl<C> CodeInterpreterInterceptionLogger<C> {
    #[must_use]
    pub fn new(
        enabled: bool,
        enabled_providers: Option<Vec<String>>,
        sandbox_tool_name: Option<String>,
        sandbox_config: Option<C>,
        catalog: Arc<ModelCatalog>,
    ) -> Self {
        Self {
            enabled,
            enabled_providers,
            sandbox_tool_name,
            sandbox_config,
            catalog,
            cache: SandboxCache::default(),
        }
    }
}

include!("initialization/tests/cases.rs");
