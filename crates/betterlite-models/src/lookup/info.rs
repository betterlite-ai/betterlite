use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::catalog::{CatalogEntry, ModelCatalog};
use crate::metadata::identity::Mode;
use crate::model_catalog_keys::{EntryProvider, check_provider_match, strip_model_name};
use crate::ptu_pricing::{MAX_COST_PER_PTU_PER_HOUR, MAX_PTU_COUNT};

use super::capabilities::CapabilityView;
use super::limits::ModelLimits;

#[derive(Clone, Copy, Debug)]
pub struct ModelQuery<'query> {
    pub model: &'query str,
    pub provider: Option<&'query str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelId<'id> {
    Integer(i64),
    Text(&'id str),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PtuBounds {
    pub ptu_count: Option<i64>,
    pub cost_per_ptu_per_hour: Option<Decimal>,
    pub ptu_effective_from: Option<DateTime<Utc>>,
    pub ptu_effective_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedPtu {
    pub count: Option<u32>,
    pub cost_per_ptu_per_hour: Option<Decimal>,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub key: String,
    pub model: String,
    pub provider: Option<String>,
    pub mode: Option<Mode>,
    pub limits: ModelLimits,
    pub capabilities: CapabilityView,
    pub ptu_count: Option<u32>,
    pub cost_per_ptu_per_hour: Option<Decimal>,
    pub ptu_effective_from: Option<DateTime<Utc>>,
    pub ptu_effective_to: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelInfoError {
    #[error(
        "This model isn't mapped yet. model={model}, custom_llm_provider={provider}. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
    )]
    Unmapped { model: String, provider: String },
    #[error("ptu_count must be a positive integer no greater than 1000000")]
    PtuCount,
    #[error("cost_per_ptu_per_hour must be a finite number between 0 and 1000000.0")]
    CostPerPtuPerHour,
    #[error("ptu_effective_to must be after ptu_effective_from")]
    PtuWindow,
}

impl ModelInfoError {
    #[must_use]
    pub fn unmapped(query: &ModelQuery<'_>) -> Self {
        Self::Unmapped {
            model: query.model.to_owned(),
            provider: query.provider.unwrap_or("None").to_owned(),
        }
    }
}

#[must_use]
pub fn find_entry<'catalog>(
    catalog: &'catalog ModelCatalog,
    query: &ModelQuery<'_>,
) -> Option<&'catalog CatalogEntry> {
    for candidate in potential_names(query) {
        let Some(entry) = lookup_entry(catalog, &candidate) else {
            continue;
        };
        if compatible(entry, query.provider) {
            return Some(entry);
        }
    }
    None
}

#[must_use]
pub fn resolved_id(id: Option<ModelId<'_>>) -> String {
    match id {
        None => uuid::Uuid::new_v4().to_string(),
        Some(ModelId::Integer(value)) => value.to_string(),
        Some(ModelId::Text(text)) => text.to_owned(),
    }
}

pub fn validated_ptu(bounds: &PtuBounds) -> Result<ValidatedPtu, ModelInfoError> {
    let count = match bounds.ptu_count {
        None => None,
        Some(count) if count <= 0 || count > i64::from(MAX_PTU_COUNT) => {
            return Err(ModelInfoError::PtuCount);
        }
        Some(count) => Some(u32::try_from(count).map_err(|_error| ModelInfoError::PtuCount)?),
    };
    if let Some(cost) = bounds.cost_per_ptu_per_hour
        && (cost < Decimal::ZERO || cost > MAX_COST_PER_PTU_PER_HOUR)
    {
        return Err(ModelInfoError::CostPerPtuPerHour);
    }
    if let (Some(start), Some(end)) = (bounds.ptu_effective_from, bounds.ptu_effective_to)
        && end <= start
    {
        return Err(ModelInfoError::PtuWindow);
    }
    Ok(ValidatedPtu {
        count,
        cost_per_ptu_per_hour: bounds.cost_per_ptu_per_hour,
        effective_from: bounds.ptu_effective_from,
        effective_to: bounds.ptu_effective_to,
    })
}

#[must_use]
pub fn declared_provider(entry: &CatalogEntry, query: &ModelQuery<'_>) -> Option<String> {
    let declared = entry.fields.metadata.litellm_provider.as_str();
    if declared.is_empty() {
        return query.provider.map(str::to_owned);
    }
    Some(declared.to_owned())
}

fn potential_names(query: &ModelQuery<'_>) -> [String; 6] {
    let provider_prefix = query.provider.map(|provider| format!("{provider}/"));
    let prefix_split = provider_prefix
        .as_deref()
        .and_then(|prefix| query.model.strip_prefix(prefix));
    match (query.provider, prefix_split) {
        (Some(provider), Some(split)) => {
            let stripped = strip_model_name(split, Some(provider), str::to_owned);
            [
                query.model.to_owned(),
                query.model.to_owned(),
                split.to_owned(),
                format!("{provider}/{stripped}"),
                stripped,
                format!("{provider}/{}", query.model),
            ]
        }
        (Some(provider), None) => {
            let combined = format!("{provider}/{}", query.model);
            let stripped = strip_model_name(query.model, Some(provider), str::to_owned);
            [
                combined.clone(),
                query.model.to_owned(),
                query.model.to_owned(),
                format!("{provider}/{stripped}"),
                stripped,
                combined,
            ]
        }
        (None, _) => {
            let stripped = strip_model_name(query.model, None, str::to_owned);
            [
                query.model.to_owned(),
                query.model.to_owned(),
                query.model.to_owned(),
                stripped.clone(),
                stripped.clone(),
                stripped,
            ]
        }
    }
}

fn lookup_entry<'catalog>(
    catalog: &'catalog ModelCatalog,
    candidate: &str,
) -> Option<&'catalog CatalogEntry> {
    if let Some(canonical) = catalog.lookup.get(candidate) {
        return catalog.entries.get(canonical.as_ref());
    }
    let lowercase = candidate.to_lowercase();
    let canonical = catalog
        .lookup
        .iter()
        .find(|(key, _)| key.to_lowercase() == lowercase)?
        .1;
    catalog.entries.get(canonical.as_ref())
}

fn compatible(entry: &CatalogEntry, provider: Option<&str>) -> bool {
    let declared = entry.fields.metadata.litellm_provider.as_str();
    let entry_provider = if declared.is_empty() {
        EntryProvider::Unconstrained
    } else {
        EntryProvider::Text(declared)
    };
    matches!(check_provider_match(entry_provider, provider), Ok(true))
}
