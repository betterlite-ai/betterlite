use std::{collections::BTreeMap, io, sync::Arc};

use crate::catalog_generalizations::CatalogGeneralizations;
use crate::entry::{ModelFieldError, ModelFields};
use crate::error::ModelsError;
use crate::metadata::identity::Mode;
use crate::price_fields::{PriceFieldError, PriceFields};

const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MAX_CATALOG_DEPTH: usize = 64;
const MAX_CATALOG_ENTRIES: usize = 10_000;
const SAMPLE_SPEC: &str = "sample_spec";
const FALLBACK_GENERALIZATIONS: &str = "fallback_generalizations";
#[expect(
    clippy::large_include_file,
    reason = "The pinned bundled catalog must be embedded byte-for-byte"
)]
const BACKUP_ASSET: &[u8] =
    include_bytes!("../fixtures/model_prices_and_context_window_backup.json");

mod decoder;
mod loader;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntry {
    pub name: String,
    pub fields: ModelFields,
    pub prices: PriceFields,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ModelMetadataProjection<'catalog> {
    pub model: &'catalog str,
    pub provider: &'catalog str,
    pub mode: Option<Mode>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("model catalog is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("model catalog has too many entries: {count} over the {limit} entry limit")]
    TooManyEntries { count: usize, limit: usize },
    #[error("model catalog JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model catalog nesting exceeds {limit}")]
    TooDeep { limit: usize },
    #[error("bundled catalog asset: {0}")]
    Asset(#[source] io::Error),
    #[error("model catalog field error for {model}: {source}")]
    Fields {
        model: String,
        source: ModelFieldError,
    },
    #[error("model catalog price error for {model}: {source}")]
    Prices {
        model: String,
        source: PriceFieldError,
    },
    #[error("model catalog fallback generalizations: {0}")]
    Generalizations(#[from] ModelsError),
}

#[derive(Debug, Default)]
pub struct ModelCatalog {
    pub entries: BTreeMap<String, CatalogEntry>,
    pub lookup: BTreeMap<String, String>,
    pub generalizations: CatalogGeneralizations,
    pub warnings: Vec<String>,
}

struct CatalogParts {
    pub entries: BTreeMap<String, CatalogEntry>,
    pub lookup: BTreeMap<String, String>,
    pub generalizations: CatalogGeneralizations,
    pub warnings: Vec<String>,
}

impl ModelCatalog {
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&CatalogEntry> {
        let canonical = self.lookup.get(name)?;
        self.entries.get(canonical)
    }
    #[must_use]
    pub fn metadata(&self, name: &str) -> Option<ModelMetadataProjection<'_>> {
        let entry = self.lookup(name)?;
        Some(ModelMetadataProjection {
            model: entry.name.as_str(),
            provider: entry.fields.metadata.litellm_provider.as_str(),
            mode: entry.fields.metadata.mode,
        })
    }
    pub fn parse(bytes: &[u8], unknown: UnknownFields) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge {
                size: bytes.len(),
                limit: MAX_CATALOG_BYTES,
            });
        }
        let raw = decoder::decode(bytes)?;
        if raw.len() > MAX_CATALOG_ENTRIES {
            return Err(CatalogError::TooManyEntries {
                count: raw.len(),
                limit: MAX_CATALOG_ENTRIES,
            });
        }
        let parts = loader::build_catalog(raw, unknown)?;
        Ok(Self {
            entries: parts.entries,
            lookup: parts.lookup,
            generalizations: parts.generalizations,
            warnings: parts.warnings,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownFields {
    Reject,
    Warn,
}

pub struct BundledCatalog<F = fn() -> Result<Vec<u8>, CatalogError>> {
    pub asset: F,
    pub cached: Option<Arc<ModelCatalog>>,
}

impl Default for BundledCatalog {
    fn default() -> Self {
        Self {
            asset: || Ok(BACKUP_ASSET.to_vec()),
            cached: None,
        }
    }
}

impl<F: Fn() -> Result<Vec<u8>, CatalogError>> BundledCatalog<F> {
    pub fn get(&mut self) -> Arc<ModelCatalog> {
        let catalog = self.cached.get_or_insert_with(|| {
            Arc::new(
                (self.asset)()
                    .and_then(|bytes| ModelCatalog::parse(&bytes, UnknownFields::Reject))
                    .unwrap_or_default(),
            )
        });
        Arc::clone(catalog)
    }
}

mod tests;
