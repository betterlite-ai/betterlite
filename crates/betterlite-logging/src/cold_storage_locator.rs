use std::sync::Arc;

use crate::cold_storage_key::{ColdStorageLocator, ColdStorageTarget};
use crate::registry::catalog::LoggerCatalog;

mod tests;

pub struct CatalogColdStorage {
    logger_name: Option<String>,
    catalog: Arc<LoggerCatalog>,
}

impl CatalogColdStorage {
    #[must_use]
    pub const fn new(logger_name: Option<String>, catalog: Arc<LoggerCatalog>) -> Self {
        Self {
            logger_name,
            catalog,
        }
    }
}

impl ColdStorageLocator for CatalogColdStorage {
    // the lookup only reads a logger the catalog already built, so nothing is constructed for a name nobody uses
    fn locate(&self) -> Option<ColdStorageTarget> {
        let name = self.logger_name.as_deref()?;
        let target = self
            .catalog
            .existing(name)
            .map_or_else(ColdStorageTarget::default, |logger| {
                logger.cold_storage_target().unwrap_or_default()
            });
        Some(target)
    }
}
