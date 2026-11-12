#[derive(Debug, thiserror::Error)]
#[error("the global {provider} registry is poisoned")]
pub struct ProviderLookupError {
    pub provider: &'static str,
}
