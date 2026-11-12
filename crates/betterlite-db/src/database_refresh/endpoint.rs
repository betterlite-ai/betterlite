use std::mem;
use std::sync::{Arc, Mutex};

use secrecy::SecretString;

// The caller publishes the URL a writer generation connects to; the pool service reads the same slot.
#[derive(Clone, Default)]
pub struct DatabaseUrlSlot {
    inner: Arc<Mutex<Option<SecretString>>>,
}

impl DatabaseUrlSlot {
    #[must_use]
    pub fn publish(&self, url: Option<SecretString>) -> Option<SecretString> {
        match self.inner.lock() {
            Ok(mut slot) => mem::replace(&mut *slot, url),
            Err(poisoned) => mem::replace(&mut *poisoned.into_inner(), url),
        }
    }

    pub(crate) fn snapshot(&self) -> Option<SecretString> {
        match self.inner.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}
