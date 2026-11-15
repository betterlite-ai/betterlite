use std::sync::Arc;

use betterlite_cache::evicted_client_closer::EvictableClient;
use betterlite_cache::llm_client_cache::{LlmClientCache, LlmClientCacheError};

// the source's f-string renders a missing project or location as Python's None
const NONE: &str = "None";

// LiteLLM's _get_client_cache_key: one unescaped hyphen concatenation over the three values
#[must_use]
pub fn cache_key(model: &str, project: Option<&str>, location: Option<&str>) -> String {
    let project = project.unwrap_or(NONE);
    let location = location.unwrap_or(NONE);
    format!("{model}-{project}-{location}")
}

// The cache read is all the source's completion does; _set_client_in_cache has no caller at the pin
pub trait ClientCache {
    type Client;

    fn cached(&self, key: &str) -> Result<Option<Arc<Self::Client>>, LlmClientCacheError>;
}

impl<Client: EvictableClient> ClientCache for LlmClientCache<Client> {
    type Client = Client;

    fn cached(&self, key: &str) -> Result<Option<Arc<Client>>, LlmClientCacheError> {
        self.get_cache(key)
    }
}
