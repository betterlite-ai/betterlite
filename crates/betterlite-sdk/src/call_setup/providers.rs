#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProviderRegistry {
    pub custom_provider_map: Vec<String>,
    pub providers: Vec<String>,
    pub custom_providers: Vec<String>,
}

// LiteLLM keeps these three registries as module globals; the caller owns them here
pub fn custom_llm_setup(registry: &mut ProviderRegistry) {
    for custom in &registry.custom_provider_map {
        if !registry.providers.contains(custom) {
            registry.providers.push(custom.clone());
        }
        if !registry.custom_providers.contains(custom) {
            registry.custom_providers.push(custom.clone());
        }
    }
}
