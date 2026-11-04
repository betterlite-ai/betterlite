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

pub struct LazyFallibleCache<T, F> {
    loader: F,
    cached: Option<T>,
}

impl<T, F> From<F> for LazyFallibleCache<T, F> {
    fn from(loader: F) -> Self {
        Self {
            loader,
            cached: None,
        }
    }
}

impl<T: Clone, E, F: FnMut() -> Result<T, E>> LazyFallibleCache<T, F> {
    // matches _get_local_model_cost_map's lru_cache: a success is cached forever, a failure is retried, never faked
    pub fn get(&mut self) -> Result<T, E> {
        if let Some(value) = &self.cached {
            return Ok(value.clone());
        }
        let value = (self.loader)()?;
        self.cached = Some(value.clone());
        Ok(value)
    }
}
