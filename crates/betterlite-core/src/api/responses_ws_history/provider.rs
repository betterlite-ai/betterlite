use thiserror::Error;

use super::{FrameSink, ManagedResponsesWebSocket};

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ProviderResolutionFailure {
    pub message: String,
}

// get_llm_provider is B174's: this seam only asks it for a model's provider name
pub trait ProviderResolution: Send + Sync {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure>;
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    #[must_use]
    pub fn resolve_provider(&self, model: Option<&str>) -> Option<String> {
        self.resolver
            .resolve(model.filter(|value| !value.is_empty())?)
            .ok()
    }

    #[must_use]
    pub fn same_provider(&self, model: Option<&str>) -> bool {
        match model {
            None => true,
            Some(model) if model == self.model.as_str() => true,
            Some(model) => {
                let Some(provider) = self.resolve_provider(Some(model)) else {
                    return false;
                };
                self.connection_provider.as_deref() == Some(provider.as_str())
            }
        }
    }
}
