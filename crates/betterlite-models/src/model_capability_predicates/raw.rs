use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::from_str;
use serde_json::value::RawValue;

use crate::decisions::Json;
use crate::model_catalog_keys::{EntryProvider, ModelCostKeys, check_provider_match};
use crate::model_metadata_resolution::names::{
    PotentialNamesInput, apply_aliases, potential_model_names,
};

use super::CapabilityEnvironment;
use super::details::{MetadataValue, Target};

// The same exact-entry candidates B2219's helper matches, read raw so a wrongly typed flag stays a value.
pub(super) fn raw_metadata<Source, Transport>(
    target: &Target,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Option<MetadataValue> {
    let mut model = target.model.clone();
    let mut provider = Some(target.provider.clone());
    apply_aliases(&mut model, &mut provider, environment.metadata.vertex);
    let names = potential_model_names(&PotentialNamesInput {
        model: &model,
        custom_llm_provider: provider.as_deref(),
        resolution: environment.metadata.resolution,
        normalizers: environment.metadata.normalizers,
    });
    for candidate in [
        names.combined_model_name.as_str(),
        model.as_str(),
        names.split_model.as_str(),
        names.combined_stripped_model_name.as_str(),
        names.stripped_model_name.as_str(),
        names.provider_prefixed_model_name.as_str(),
    ] {
        let Some(entry_key) = environment
            .keys
            .get_model_cost_key(environment.metadata.model_cost, candidate)
        else {
            continue;
        };
        let Some(raw) = environment.metadata.model_cost.get(&entry_key) else {
            continue;
        };
        // an unreadable entry stops the lookup, the way B2219's exact entry re-raises it as not-mapped
        let Ok(fields) = from_str::<BTreeMap<String, Box<RawValue>>>(raw.get()) else {
            return None;
        };
        // only a clean provider mismatch skips to the next candidate; a failed check stops like B2219
        match check_provider_match(entry_provider(&fields), provider.as_deref()) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return None,
        }
        let Some(value) = fields.get(key).filter(|value| value.get() != "null") else {
            return Some(MetadataValue::Absent);
        };
        return from_str::<Json>(value.get()).ok().map(MetadataValue::Value);
    }
    None
}

fn entry_provider(fields: &BTreeMap<String, Box<RawValue>>) -> EntryProvider<'_> {
    match fields.get("litellm_provider") {
        None => EntryProvider::Unconstrained,
        Some(value) if value.get() == "null" => EntryProvider::Unconstrained,
        Some(value) => {
            from_str::<&str>(value.get()).map_or(EntryProvider::NotText, EntryProvider::Text)
        }
    }
}

// The unprefixed twin of a provider-prefixed entry, which the map may declare instead (#20885).
fn bare_field(
    model_cost: &IndexMap<String, Box<RawValue>>,
    keys: &mut ModelCostKeys,
    model: &str,
    key: &str,
) -> Option<Box<RawValue>> {
    let entry_key = keys.get_model_cost_key(model_cost, model)?;
    let entry = model_cost.get(&entry_key)?;
    let fields: BTreeMap<String, Box<RawValue>> = from_str(entry.get()).ok()?;
    let value = fields.get(key)?;
    // an unreadable entry answers nothing, which is what LiteLLM's dict check on the bare entry yields
    (value.get() != "null").then(|| value.clone())
}

// A bare entry that is not an object answers nothing, the way LiteLLM's isinstance check skips it.
pub(super) fn bare_declaration<Source, Transport>(
    model: &str,
    key: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Option<Json> {
    let metadata = &environment.metadata;
    let value = bare_field(metadata.model_cost, &mut *environment.keys, model, key)?;
    from_str(value.get()).ok()
}
