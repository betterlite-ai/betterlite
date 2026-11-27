use std::sync::{Mutex, PoisonError};

use betterlite_auth::identity_store::IdentityStore;
use betterlite_auth::identity_store::error::IdentityStoreError;
use betterlite_auth::identity_store::ports::{Dependencies, Ports};
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// The composition root installs the reader, cache, permission and writer ports once the proxy owns them.
pub struct Slot {
    dependencies: Mutex<Option<Dependencies>>,
}

impl Slot {
    #[must_use]
    pub const fn new(_config: &Config, _shared: &Shared) -> Self {
        Self {
            dependencies: Mutex::new(None),
        }
    }

    // A port left out is named here, at startup, instead of failing an authentication later.
    pub fn install(&self, ports: Ports) -> Result<(), IdentityStoreError> {
        let dependencies = ports.require()?;
        *self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(dependencies);
        Ok(())
    }

    // The stage resolves in the store's normal mode with no parent span until a caller supplies one.
    #[must_use]
    pub fn store(&self) -> Option<IdentityStore> {
        let dependencies = self
            .dependencies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        dependencies.map(|dependencies| IdentityStore::new(dependencies, None, false))
    }
}
