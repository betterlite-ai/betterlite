use std::sync::{Arc, Mutex, PoisonError};

use crate::spend::runtime::parts::SpendRuntimeParts;
use crate::spend::runtime::{SpendRuntime, SpendRuntimeError, SpendRuntimeSettings};
use crate::state::shared::Shared;
use betterlite_config::types::Config;

pub struct Slot {
    pub runtime: Mutex<Option<Arc<SpendRuntime>>>,
}

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        // the state is built before the database and catalog ports exist, so the composition root installs the runtime here
        Self {
            runtime: Mutex::new(None),
        }
    }

    // Installation is what makes the runtime's queues and flush handles single: a second one would take the first one's work.
    pub fn install(
        &self,
        settings: SpendRuntimeSettings,
        parts: SpendRuntimeParts,
    ) -> Result<Arc<SpendRuntime>, SpendRuntimeError> {
        let mut installed = self.runtime.lock().unwrap_or_else(PoisonError::into_inner);
        if installed.is_some() {
            return Err(SpendRuntimeError::AlreadyInstalled);
        }
        let runtime = SpendRuntime::new(settings, parts)?;
        *installed = Some(Arc::clone(&runtime));
        drop(installed);
        Ok(runtime)
    }
}
