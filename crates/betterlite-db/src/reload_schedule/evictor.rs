use std::future::Future;
use std::pin::Pin;

pub type EvictFuture<'name> = Pin<Box<dyn Future<Output = ()> + Send + 'name>>;

// LiteLLM's evict_config_param drops this process's cached row; invalidate_config_param adds the publish step
pub trait ConfigParamEvictor: Send + Sync {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call>;
}
