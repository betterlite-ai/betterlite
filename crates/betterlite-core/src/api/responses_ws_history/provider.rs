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

#[must_use]
pub fn resolve_provider<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    model: Option<&str>,
) -> Option<String>
where
    Sink: FrameSink,
{
    if model.is_none_or(str::is_empty) {
        return None;
    }
    handler.resolver.resolve(model?).ok()
}

#[must_use]
pub fn same_provider<Sink>(handler: &ManagedResponsesWebSocket<Sink>, model: Option<&str>) -> bool
where
    Sink: FrameSink,
{
    let Some(candidate) = model else {
        return true;
    };
    if candidate == handler.model.as_str() {
        return true;
    }
    let Some(provider) = resolve_provider(handler, Some(candidate)) else {
        return false;
    };
    handler.connection_provider.as_deref() == Some(provider.as_str())
}
