use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};

use crate::catalog_generalizations::CatalogGeneralizations;
use crate::model_catalog_keys::{
    EntryProvider, ModelCostKeys, PotentialModelNames, check_provider_match,
    get_model_info_from_model_cost,
};
use crate::model_metadata_projection::project;

use super::error::StaticFailure;

const BACKFILL_MODES: [&str; 2] = ["chat", "responses"];
const PROVIDER_KEY: &str = "litellm_provider";

pub(super) struct Lookup<'lookup> {
    pub(super) model: &'lookup str,
    pub(super) provider: Option<&'lookup str>,
    pub(super) names: &'lookup PotentialModelNames,
    pub(super) model_cost: &'lookup IndexMap<String, Box<RawValue>>,
    pub(super) generalizations: &'lookup CatalogGeneralizations,
}

pub(super) struct MatchedEntry {
    pub(super) key: String,
    pub(super) entry: Box<RawValue>,
}

fn candidates<'names>(model: &'names str, names: &'names PotentialModelNames) -> [&'names str; 6] {
    [
        &names.combined_model_name,
        model,
        &names.split_model,
        &names.combined_stripped_model_name,
        &names.stripped_model_name,
        &names.provider_prefixed_model_name,
    ]
}

pub(super) fn exact_entry(
    lookup: &Lookup<'_>,
    keys: &mut ModelCostKeys,
) -> Result<Option<MatchedEntry>, StaticFailure> {
    for candidate in candidates(lookup.model, lookup.names) {
        if let Some(matched) = exact_candidate(lookup, candidate, keys)? {
            return Ok(Some(matched));
        }
    }
    Ok(None)
}

fn exact_candidate(
    lookup: &Lookup<'_>,
    candidate: &str,
    keys: &mut ModelCostKeys,
) -> Result<Option<MatchedEntry>, StaticFailure> {
    let Some(key) = keys.get_model_cost_key(lookup.model_cost, candidate) else {
        return Ok(None);
    };
    let raw = get_model_info_from_model_cost(lookup.model_cost, &key)?;
    let fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(raw.get())?;
    if !check_provider_match(entry_provider(&fields), lookup.provider)? {
        return Ok(None);
    }
    let entry = backfill(&fields, &key, lookup.generalizations)?;
    Ok(Some(MatchedEntry { key, entry }))
}

fn entry_provider(fields: &BTreeMap<String, Box<RawValue>>) -> EntryProvider<'_> {
    match fields.get(PROVIDER_KEY) {
        None => EntryProvider::Unconstrained,
        Some(value) if value.get() == "null" => EntryProvider::Unconstrained,
        Some(value) => serde_json::from_str::<&str>(value.get())
            .map_or(EntryProvider::NotText, EntryProvider::Text),
    }
}

// LiteLLM backfills only an exact entry in chat or responses mode, and only with keys the entry lacks.
fn backfill(
    fields: &BTreeMap<String, Box<RawValue>>,
    key: &str,
    generalizations: &CatalogGeneralizations,
) -> Result<Box<RawValue>, StaticFailure> {
    let Some(filled) = fill_missing(fields, key, generalizations) else {
        return to_raw_value(fields).map_err(StaticFailure::from);
    };
    let mut merged = filled;
    for (name, value) in fields {
        merged.insert(name.clone(), value.clone());
    }
    to_raw_value(&merged).map_err(StaticFailure::from)
}

fn fill_missing(
    fields: &BTreeMap<String, Box<RawValue>>,
    key: &str,
    generalizations: &CatalogGeneralizations,
) -> Option<BTreeMap<String, Box<RawValue>>> {
    if !fill_mode(fields) {
        return None;
    }
    let provider = serde_json::from_str::<&str>(fields.get(PROVIDER_KEY)?.get()).ok()?;
    generalizations.match_fill_missing(key, provider)
}

fn fill_mode(fields: &BTreeMap<String, Box<RawValue>>) -> bool {
    fields.get("mode").is_none_or(|value| {
        serde_json::from_str::<&str>(value.get()).is_ok_and(|mode| BACKFILL_MODES.contains(&mode))
    })
}

pub(super) fn generalization(
    lookup: &Lookup<'_>,
    keys: &mut ModelCostKeys,
) -> Result<Option<MatchedEntry>, StaticFailure> {
    let candidates = candidates(lookup.model, lookup.names);
    // A known name, even for another provider, is not unmapped: rules must not shadow a cost entry.
    if candidates.iter().any(|candidate| {
        keys.get_model_cost_key(lookup.model_cost, candidate)
            .is_some()
    }) {
        return Ok(None);
    }
    let matched = candidates.iter().find_map(|candidate| {
        lookup
            .generalizations
            .match_capabilities(candidate)
            .map(|info| (candidate, info))
    });
    let Some((candidate, mut info)) = matched else {
        return Ok(None);
    };
    if let Some(provider) = lookup.provider {
        info.insert(PROVIDER_KEY.to_owned(), to_raw_value(provider)?);
    }
    Ok(Some(MatchedEntry {
        key: (*candidate).to_owned(),
        entry: to_raw_value(&info)?,
    }))
}

pub(super) fn project_entry(
    matched: &MatchedEntry,
    model: &str,
    provider: Option<&str>,
) -> Result<Box<RawValue>, StaticFailure> {
    let Some(requested) = provider else {
        // B2218's project takes a text fallback; LiteLLM keeps null when no provider was requested
        let entry = without_provider(matched)?;
        return project(&matched.key, model, "", &entry).map_err(StaticFailure::from);
    };
    project(&matched.key, model, requested, &matched.entry).map_err(StaticFailure::from)
}

fn without_provider(matched: &MatchedEntry) -> Result<Box<RawValue>, StaticFailure> {
    let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(matched.entry.get())?;
    if !fields.contains_key(PROVIDER_KEY) {
        let null = RawValue::from_string(String::from("null"))?;
        fields.insert(PROVIDER_KEY.to_owned(), null);
    }
    to_raw_value(&fields).map_err(StaticFailure::from)
}
