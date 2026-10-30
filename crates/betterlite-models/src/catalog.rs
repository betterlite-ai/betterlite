use std::collections::BTreeMap;

use serde_json::value::RawValue;

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

mod decoder;
mod loader;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogEntry {
    pub name: String,
    pub fields: ModelFields,
    pub prices: PriceFields,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    #[error("model catalog syntax error: {0}")]
    Scan(String),
    #[error("model catalog duplicate key at {path}: {key}")]
    DuplicateKey { path: String, key: String },
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

#[derive(Debug)]
pub struct ModelCatalog {
    entries: BTreeMap<String, CatalogEntry>,
    lookup: BTreeMap<String, String>,
    generalizations: CatalogGeneralizations,
    warnings: Vec<String>,
}

struct CatalogParts {
    entries: BTreeMap<String, CatalogEntry>,
    lookup: BTreeMap<String, String>,
    generalizations: CatalogGeneralizations,
    warnings: Vec<String>,
}

impl ModelCatalog {
    pub fn parse_bundled(bytes: &[u8]) -> Result<Self, CatalogError> {
        let strict = true;
        Self::parse(bytes, strict)
    }
    pub fn parse_remote(bytes: &[u8]) -> Result<Self, CatalogError> {
        let strict = false;
        Self::parse(bytes, strict)
    }
    pub fn bundled() -> Result<Self, CatalogError> {
        let mut bytes = Vec::new();
        for part in BACKUP_PARTS {
            bytes.extend_from_slice(part);
        }
        Self::parse_bundled(&bytes)
    }
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&CatalogEntry> {
        let canonical = self.lookup.get(name)?;
        self.entries.get(canonical)
    }
    #[must_use]
    pub fn canonical_name(&self, name: &str) -> Option<&str> {
        if name.is_empty() {
            return None;
        }
        self.lookup.get(name).map(String::as_str)
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
    #[must_use]
    pub fn len(&self) -> usize {
        if self.entries.is_empty() {
            return 0;
        }
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        if self.entries.is_empty() {
            return true;
        }
        false
    }
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        if self.warnings.is_empty() {
            return &[];
        }
        &self.warnings
    }
    #[must_use]
    pub const fn generalizations(&self) -> &CatalogGeneralizations {
        if self.generalizations.rules.is_empty() {
            return &self.generalizations;
        }
        &self.generalizations
    }

    fn parse(bytes: &[u8], strict: bool) -> Result<Self, CatalogError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge {
                size: bytes.len(),
                limit: MAX_CATALOG_BYTES,
            });
        }
        decoder::check_duplicates(bytes)?;
        let raw: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
        if raw.len() > MAX_CATALOG_ENTRIES {
            return Err(CatalogError::TooManyEntries {
                count: raw.len(),
                limit: MAX_CATALOG_ENTRIES,
            });
        }
        let parts = loader::build_catalog(raw, strict)?;
        Ok(Self {
            entries: parts.entries,
            lookup: parts.lookup,
            generalizations: parts.generalizations,
            warnings: parts.warnings,
        })
    }
}

const BACKUP_PARTS: &[&[u8]] = &[
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-aa"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ab"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ac"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ad"),
];

mod tests;
