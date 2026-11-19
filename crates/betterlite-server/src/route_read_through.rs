use async_trait::async_trait;
use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;

use crate::registry_read_through::{RegistryReadThrough, Resync};

mod tests;

#[async_trait]
pub trait ModelReadThrough: Send + Sync {
    async fn attempt(&self, model: &str) -> bool;
}

#[async_trait]
impl<R: Resync> ModelReadThrough for RegistryReadThrough<R> {
    async fn attempt(&self, model: &str) -> bool {
        RegistryReadThrough::attempt(self, model).await
    }
}

pub trait ModelNotFoundCause {
    fn model_not_found(&self) -> Option<&ProxyModelNotFoundError>;
}

impl ModelNotFoundCause for ProxyModelNotFoundError {
    fn model_not_found(&self) -> Option<&ProxyModelNotFoundError> {
        Some(self)
    }
}

pub async fn route_with_read_through<T, E, F>(
    model: Option<&str>,
    read_through: Option<&dyn ModelReadThrough>,
    attempt: F,
) -> Result<T, E>
where
    E: ModelNotFoundCause,
    F: AsyncFn() -> Result<T, E>,
{
    let first = match attempt().await {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let (Some(model_name), Some(read_through_port)) = (model, read_through) else {
        return Err(first);
    };
    if model_name.is_empty() {
        return Err(first);
    }
    // LiteLLM retries only a not-found whose read-through flag is on
    let retryable = first.model_not_found().is_some_and(|not_found| {
        // ast-grep-ignore: no-long-names
        not_found.retryable_with_model_read_through
    });
    if !retryable {
        return Err(first);
    }
    if !read_through_port.attempt(model_name).await {
        return Err(first);
    }
    attempt().await
}
