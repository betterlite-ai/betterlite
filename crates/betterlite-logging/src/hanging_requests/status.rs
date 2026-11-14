use async_trait::async_trait;
use betterlite_models::decisions::Json;

#[async_trait]
pub trait CompletionStatusCache: Send + Sync {
    async fn local_get(&self, key: &str) -> Option<Json>;
}
