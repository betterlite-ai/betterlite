use std::sync::{Arc, Mutex};

// Records what the fakes saw in the order they saw it; every collaborator of one harness shares it
#[derive(Clone)]
pub(crate) struct Journal<T>(Arc<Mutex<Vec<T>>>);

impl<T> Default for Journal<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl<T: Clone> Journal<T> {
    pub(crate) fn record(&self, entry: T) {
        match self.0.lock() {
            Ok(mut entries) => entries.push(entry),
            Err(poisoned) => poisoned.into_inner().push(entry),
        }
    }

    pub(crate) fn entries(&self) -> Vec<T> {
        match self.0.lock() {
            Ok(entries) => entries.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Alert,
    SuccessSpan,
    FailureSpan,
    Delete(String),
}
