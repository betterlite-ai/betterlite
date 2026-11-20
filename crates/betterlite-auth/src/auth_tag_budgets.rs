pub mod cache;
pub mod groups;
pub mod postgres;
pub mod registry;
pub mod rows;
pub mod tags;
mod tests;

use std::collections::BTreeMap;

use crate::auth_cache::payload::PayloadError;
use crate::end_user_registry::errors::RegistryError;

// Every failure a batch read raises: a cache read or the tag registry; fetch failures fail open instead.
#[derive(Debug, thiserror::Error)]
pub enum TagBudgetError {
    #[error("the cached tag payload could not be read")]
    TagPayload(#[source] Box<PayloadError>),
    #[error("the cached model access group budget payload could not be read")]
    GroupPayload(#[source] Box<PayloadError>),
    #[error(transparent)]
    Registry(#[from] RegistryError),
}

// LiteLLM spreads the probes first and the fetched rows last, so a fetched row wins over a same-name hit.
pub(super) fn merged<Value>(
    probed: Vec<(String, Option<Value>)>,
    fetched: Vec<(String, Value)>,
) -> BTreeMap<String, Value> {
    probed
        .into_iter()
        .filter_map(|(name, cached)| cached.map(|present| (name, present)))
        .chain(fetched)
        .collect()
}
