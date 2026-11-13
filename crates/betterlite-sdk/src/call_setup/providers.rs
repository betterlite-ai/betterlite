#[derive(Debug, Default)]
pub struct ProviderRegistry {
    pub custom_provider_map: Vec<String>,
    pub providers: Vec<String>,
    pub custom_providers: Vec<String>,
}

pub fn custom_llm_setup(registry: &mut ProviderRegistry) {
    for provider in &registry.custom_provider_map {
        if !registry.providers.contains(provider) {
            registry.providers.push(provider.clone());
        }
        if !registry.custom_providers.contains(provider) {
            registry.custom_providers.push(provider.clone());
        }
    }
}
