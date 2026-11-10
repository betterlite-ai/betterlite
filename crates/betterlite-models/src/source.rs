use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};

use crate::catalog::ModelCatalog;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceKind {
    #[default]
    Local,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogSource {
    Local,
    Remote { url: String },
}

impl CatalogSource {
    #[must_use]
    pub fn from_settings(local: bool, url: &str) -> Self {
        if local {
            return Self::Local;
        }
        Self::Remote {
            url: url.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogProvenance {
    pub kind: SourceKind,
    pub url: Option<String>,
    pub is_env_forced: bool,
    pub fallback_reason: Option<String>,
    pub loaded_at: Option<DateTime<Utc>>,
    pub source_revision: Option<String>,
    pub etag: Option<String>,
}

pub type CatalogClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct CatalogSnapshot {
    pub catalog: Arc<ModelCatalog>,
    pub provenance: CatalogProvenance,
}

// Readers take the whole snapshot or the previous one; a replacement never leaves a mixed pair
#[derive(Default)]
pub struct CatalogStore {
    current: RwLock<Arc<CatalogSnapshot>>,
}

impl CatalogStore {
    #[must_use]
    pub fn load(&self) -> Arc<CatalogSnapshot> {
        match self.current.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    pub(crate) fn store(&self, snapshot: Arc<CatalogSnapshot>) {
        match self.current.write() {
            Ok(mut guard) => *guard = snapshot,
            Err(poisoned) => *poisoned.into_inner() = snapshot,
        }
    }
}

mod tests;
