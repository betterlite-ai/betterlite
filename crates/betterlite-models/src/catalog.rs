use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::catalog_generalizations::{CatalogGeneralizations, decode_generalization_rules};
use crate::entry::generalizations::FallbackGeneralizations;
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
        let parts = build_catalog(raw, strict)?;
        Ok(Self {
            entries: parts.entries,
            lookup: parts.lookup,
            generalizations: parts.generalizations,
            warnings: parts.warnings,
        })
    }
}

fn build_catalog(
    raw: BTreeMap<String, Box<RawValue>>,
    strict: bool,
) -> Result<CatalogParts, CatalogError> {
    let mut entries = BTreeMap::new();
    let mut lookup = BTreeMap::new();
    let mut warnings = Vec::new();
    let (mut generalizations, mut alias_claims) = (CatalogGeneralizations::default(), Vec::new());
    for (name, value) in raw {
        if name == SAMPLE_SPEC {
            continue;
        }
        if name == FALLBACK_GENERALIZATIONS {
            generalizations = install_generalizations(value.as_ref(), &mut warnings)?;
            continue;
        }
        let fields_map: BTreeMap<String, Box<RawValue>> = serde_json::from_str(value.get())?;
        let typed_value = without_aliases(&fields_map)?;
        let fields = parse_fields(&name, typed_value.as_ref(), strict, &mut warnings)?;
        let prices =
            PriceFields::parse(typed_value.as_ref()).map_err(|source| CatalogError::Prices {
                model: name.clone(),
                source,
            })?;
        entries.insert(
            name.clone(),
            CatalogEntry {
                name: name.clone(),
                fields,
                prices,
            },
        );
        insert_lookup(&mut lookup, &name, &name, &mut warnings);
        add_alias_claims(&name, &fields_map, &mut warnings, &mut alias_claims);
    }
    for (alias, canonical) in alias_claims {
        insert_lookup(&mut lookup, &alias, &canonical, &mut warnings);
    }
    Ok(CatalogParts {
        entries,
        lookup,
        generalizations,
        warnings,
    })
}

fn add_alias_claims(
    name: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
    claims: &mut Vec<(String, String)>,
) {
    let aliases = aliases(name, fields, warnings);
    claims.extend(aliases.into_iter().map(|alias| (alias, name.to_owned())));
}

fn install_generalizations(
    value: &RawValue,
    warnings: &mut Vec<String>,
) -> Result<CatalogGeneralizations, CatalogError> {
    let declaration: FallbackGeneralizations = serde_json::from_str(value.get())?;
    let decoded = decode_generalization_rules(&serde_json::to_vec(&declaration.rules)?)?;
    let (installed, install_warnings) = CatalogGeneralizations::install(decoded);
    warnings.extend(install_warnings.into_iter().map(|error| error.to_string()));
    Ok(installed)
}

const BACKUP_PARTS: &[&[u8]] = &[
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-aa"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ab"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ac"),
    include_bytes!("../fixtures/model_prices_and_context_window_backup.part-ad"),
];

fn parse_fields(
    name: &str,
    value: &RawValue,
    strict: bool,
    warnings: &mut Vec<String>,
) -> Result<ModelFields, CatalogError> {
    let parsed = if strict {
        ModelFields::parse_strict(value)
    } else {
        let (fields, paths) = ModelFields::parse(value).map_err(|source| CatalogError::Fields {
            model: name.to_owned(),
            source,
        })?;
        warnings.extend(paths.into_iter().map(|path| format!("{name}.{path}")));
        Ok(fields)
    };
    parsed.map_err(|source| CatalogError::Fields {
        model: name.to_owned(),
        source,
    })
}

fn without_aliases(
    fields: &BTreeMap<String, Box<RawValue>>,
) -> Result<Box<RawValue>, CatalogError> {
    let typed_fields = fields
        .iter()
        .filter(|(key, _)| key.as_str() != "aliases")
        .map(|(key, value)| (key.clone(), value))
        .collect::<BTreeMap<_, _>>();
    Ok(serde_json::from_slice(&serde_json::to_vec(&typed_fields)?)?)
}

fn aliases(
    model: &str,
    fields: &BTreeMap<String, Box<RawValue>>,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(raw) = fields.get("aliases") else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(raw.get()) {
        Ok(values) => values,
        Err(error) => {
            warnings.push(format!("{model}.aliases: {error}"));
            Vec::new()
        }
    }
}

fn insert_lookup(
    lookup: &mut BTreeMap<String, String>,
    key: &str,
    canonical: &str,
    warnings: &mut Vec<String>,
) {
    if let Some(previous) = lookup.get(key) {
        warnings.push(format!(
            "alias collision for {key}: kept {previous}, ignored {canonical}"
        ));
    } else {
        lookup.insert(key.to_owned(), canonical.to_owned());
    }
}

mod tests;
