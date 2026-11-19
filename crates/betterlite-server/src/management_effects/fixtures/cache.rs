use crate::management_effects::eviction::ManagementCacheEviction;

use super::journal::{Effect, Journal};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FakeCacheError {
    #[error("the fake cache refused to delete {key}")]
    Refused { key: String },
}

#[derive(Clone, Default)]
pub(crate) struct RecordingCache {
    pub(crate) events: Journal<Effect>,
    pub(crate) deletes: Journal<String>,
    pub(crate) failing: Option<String>,
}

impl ManagementCacheEviction for RecordingCache {
    type Error = FakeCacheError;

    fn delete_cache(&self, key: &str) -> Result<(), Self::Error> {
        self.deletes.record(key.to_owned());
        self.events.record(Effect::Delete(key.to_owned()));
        if self.failing.as_deref() == Some(key) {
            return Err(FakeCacheError::Refused {
                key: key.to_owned(),
            });
        }
        Ok(())
    }
}
