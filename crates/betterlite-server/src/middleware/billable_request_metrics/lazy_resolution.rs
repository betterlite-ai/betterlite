mod tests;

use std::sync::{Arc, Mutex, PoisonError};

type Factory<T> = Box<dyn Fn() -> Option<Arc<T>> + Send + Sync>;

struct LazyState<T: ?Sized> {
    resolved: bool,
    value: Option<Arc<T>>,
}

// Double-checked locking: the slow path re-checks `resolved` under the lock so concurrent first requests block on one another instead of each building their own instance, and caches a `None` result the same as a resolved value.
pub(super) struct LazyResolve<T: ?Sized> {
    factory: Option<Factory<T>>,
    state: Mutex<LazyState<T>>,
}

impl<T: ?Sized> LazyResolve<T> {
    pub(super) fn new(factory: Option<Factory<T>>) -> Self {
        let resolved = factory.is_none();
        Self {
            factory,
            state: Mutex::new(LazyState {
                resolved,
                value: None,
            }),
        }
    }

    pub(super) fn get(&self) -> Option<Arc<T>> {
        {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.resolved {
                return state.value.clone();
            }
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.resolved {
            state.value = self.factory.as_ref().and_then(|factory| factory());
            state.resolved = true;
        }
        state.value.clone()
    }
}
