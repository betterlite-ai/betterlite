use std::error::Error;
use std::future::Future;
use std::pin::Pin;

pub type EvictFuture<'name> = Pin<Box<dyn Future<Output = Result<(), EvictError>> + Send + 'name>>;

// evict_config_param raises when the cache cannot drop the row, and every write helper awaits it
#[derive(Debug, thiserror::Error)]
#[error("the cached {param_name} row could not be evicted")]
pub struct EvictError {
    pub param_name: String,
    #[source]
    pub source: Box<dyn Error + Send + Sync>,
}

// LiteLLM's evict_config_param drops this process's cached row; invalidate_config_param adds the publish step
pub trait ConfigParamEvictor: Send + Sync {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call>;
}
