pub trait ProviderCapabilities: Send + Sync {
    fn handles_web_search_natively(&self, model: &str, provider: &str) -> bool;

    fn supports_rich_search_input(&self, search_provider: &str) -> bool;
}
