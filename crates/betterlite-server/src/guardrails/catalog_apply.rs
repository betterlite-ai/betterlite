mod airlines;
mod apply;
pub mod blocked_words;
pub mod body;
mod category;
pub mod error;
mod input_type;
pub mod ports;
mod success_logs;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::registry_read_through::RegistryReadThrough;
use crate::registry_read_through::Resync;
use crate::registry_read_through::lookup::GuardrailLookup;

use ports::ApplyPorts;

// B8668 builds this once and mounts routes(); the roots are LiteLLM's content_filter_data_roots
pub struct CatalogApplyState<L, R: Resync> {
    pub ports: Arc<ApplyPorts>,
    pub lookup: Arc<L>,
    pub read_through: Arc<RegistryReadThrough<R>>,
    pub roots: Arc<[PathBuf]>,
}

impl<L, R: Resync> Clone for CatalogApplyState<L, R> {
    fn clone(&self) -> Self {
        Self {
            ports: Arc::clone(&self.ports),
            lookup: Arc::clone(&self.lookup),
            read_through: Arc::clone(&self.read_through),
            roots: Arc::clone(&self.roots),
        }
    }
}

pub fn routes<L, R>(state: CatalogApplyState<L, R>) -> Router
where
    L: GuardrailLookup + 'static,
    R: Resync + 'static,
{
    let roots = Arc::clone(&state.roots);
    Router::new()
        .route(airlines::AIRLINES_PATH, get(airlines::major_airlines))
        .route(
            blocked_words::VALIDATE_PATH,
            post(blocked_words::validate_blocked_words_file),
        )
        .merge(apply::routes(state))
        .merge(category::routes(roots))
}
