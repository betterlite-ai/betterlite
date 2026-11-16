use thiserror::Error;

#[derive(Debug, Error)]
pub enum PromptRegistryError {
    #[error("Unsupported prompt: {integration}")]
    UnsupportedIntegration { integration: String },
}
