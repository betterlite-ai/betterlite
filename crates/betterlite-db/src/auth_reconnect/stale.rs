use std::sync::Arc;

use crate::database_refresh::generation::Observation;

pub struct StaleReadEngine<Handle> {
    pub(crate) handle: Arc<Handle>,
    pub(crate) generation: u64,
}

impl<Handle> StaleReadEngine<Handle> {
    pub fn observe(observation: &Observation<Handle>) -> Option<Self> {
        let handle = observation.handle.as_ref()?;
        Some(Self {
            handle: Arc::clone(handle),
            generation: observation.generation,
        })
    }

    #[must_use]
    pub fn is_still_live(&self, current: &Observation<Handle>) -> bool {
        current
            .handle
            .as_ref()
            .is_some_and(|handle| Arc::ptr_eq(handle, &self.handle))
            && current.generation == self.generation
    }
}
