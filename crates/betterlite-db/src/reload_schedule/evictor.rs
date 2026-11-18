use std::future::Future;
use std::pin::Pin;

pub type EvictFuture<'name> = Pin<Box<dyn Future<Output = Result<(), EvictError>> + Send + 'name>>;

// evict_config_param raises when the cache cannot drop the row, and every write helper awaits it
#[derive(Debug, thiserror::Error)]
pub enum EvictError {
    #[error("the cached {param_name} row could not be evicted: {source}")]
    Io {
        param_name: String,
        #[source]
        source: std::io::Error,
    },
}

// LiteLLM's evict_config_param drops this process's cached row; invalidate_config_param adds the publish step
pub trait ConfigParamEvictor: Send + Sync {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call>;
}
