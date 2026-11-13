use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::catalog::{CatalogError, ModelCatalog, UnknownFields};

pub static LOCAL_MODEL_COST_MAP: Mutex<Option<Arc<ModelCatalog>>> = Mutex::new(None);

// matches _get_local_model_cost_map's lru_cache(maxsize=1): a success is cached forever, a failure is retried.
pub fn cached_catalog<F>(
    cache: &Mutex<Option<Arc<ModelCatalog>>>,
    asset: F,
) -> Result<Arc<ModelCatalog>, CatalogError>
where
    F: FnOnce() -> Result<Vec<u8>, CatalogError>,
{
    let mut slot = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(catalog) = slot.as_ref() {
        return Ok(Arc::clone(catalog));
    }
    let bytes = asset()?;
    let catalog = Arc::new(ModelCatalog::parse(&bytes, UnknownFields::Reject)?);
    *slot = Some(Arc::clone(&catalog));
    drop(slot);
    Ok(catalog)
}

pub struct LazyBinding<F> {
    binder: fn() -> F,
    cached: Option<F>,
}

impl<F> From<fn() -> F> for LazyBinding<F> {
    fn from(binder: fn() -> F) -> Self {
        Self {
            binder,
            cached: None,
        }
    }
}

impl<F: Copy> LazyBinding<F> {
    // matches get_cached_model_info: the import is cached, calling the bound value stays the caller's job
    pub fn get(&mut self) -> F {
        if let Some(value) = self.cached {
            return value;
        }
        let value = (self.binder)();
        self.cached = Some(value);
        value
    }
}
