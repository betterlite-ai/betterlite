use crate::sandbox_lifecycle::cache_operations::SandboxCache;

// LiteLLM's CodeInterpreterInterceptionLogger holds the deployment's settings and the sandbox cache a call shares with its follow-ups
pub struct CodeInterpreterInterceptionLogger<C> {
    pub enabled: bool,
    pub enabled_providers: Option<Vec<String>>,
    pub sandbox_tool_name: Option<String>,
    pub sandbox_config: Option<C>,
    pub cache: SandboxCache,
}

include!("initialization/tests/cases.rs");
