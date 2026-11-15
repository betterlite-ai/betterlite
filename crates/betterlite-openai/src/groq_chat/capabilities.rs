use thiserror::Error;

#[derive(Debug, Error)]
#[error("the {name} capability lookup for {model} failed: {message}")]
pub struct CapabilityError {
    pub name: &'static str,
    pub model: String,
    pub message: String,
}

// B2221 owns the catalog reads behind these; a failing lookup is reported so the caller decides between a debug log and an error
pub trait GroqCapabilities {
    fn supports_web_search(&self, model: &str, provider: &str) -> Result<bool, CapabilityError>;
    fn supports_reasoning(&self, model: &str, provider: &str) -> Result<bool, CapabilityError>;
    fn supports_response_schema(
        &self,
        model: &str,
        provider: &str,
    ) -> Result<bool, CapabilityError>;
}
