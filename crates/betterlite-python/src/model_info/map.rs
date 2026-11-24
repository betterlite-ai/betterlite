use super::error::ModelInfoError;
use betterlite_models::catalog_generalizations::decode_generalization_rules;
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::integrity::RESERVED_TOP_LEVEL_KEYS;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use std::collections::BTreeSet;
const FALLBACK_KEY: &str = "fallback_generalizations";
const ALIASES_KEY: &str = "aliases";
pub(crate) struct FinalizedMap {
    pub(crate) entries: IndexMap<String, Box<RawValue>>,
    pub(crate) rules: Vec<Box<RawValue>>,
}
// LiteLLM's _finalize_model_cost_map: pull the fallback rules out of the map, then expand aliases into entries.
pub(crate) fn finalize(
    mut entries: IndexMap<String, Box<RawValue>>,
) -> Result<FinalizedMap, ModelInfoError> {
    let rules = match entries.shift_remove(FALLBACK_KEY) {
        Some(block) => rules_from(&block)?,
        None => Vec::new(),
    };
    let canonical: BTreeSet<String> = entries.keys().cloned().collect();
    let mut claims: Vec<(String, Box<RawValue>)> = Vec::new();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for entry in entries.values_mut() {
        let Ok(Json::Object(mut fields)) = serde_json::from_str::<Json>(entry.get()) else {
            continue;
        };
        let aliases = match fields.shift_remove(ALIASES_KEY) {
            None => continue,
            Some(Json::List(aliases)) => aliases,
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            ) => Vec::new(),
        };
        let text = serde_json::to_string(&fields).map_err(ModelInfoError::Result)?;
        *entry = RawValue::from_string(text).map_err(ModelInfoError::Result)?;
        for alias in aliases {
            let Json::Text(name) = alias else {
                continue;
            };
            if canonical.contains(&name) || !claimed.insert(name.clone()) {
                continue;
            }
            claims.push((name, entry.clone()));
        }
    }
    for (alias, entry) in claims {
        drop(entries.insert(alias, entry));
    }
    Ok(FinalizedMap { entries, rules })
}
fn rules_from(block: &RawValue) -> Result<Vec<Box<RawValue>>, ModelInfoError> {
    let Ok(Json::Object(mut fields)) = serde_json::from_str::<Json>(block.get()) else {
        return Ok(Vec::new());
    };
    let Some(rules) = fields.shift_remove("rules") else {
        return Ok(Vec::new());
    };
    let text = serde_json::to_string(&rules).map_err(ModelInfoError::Result)?;
    decode_generalization_rules(text.as_bytes()).map_err(ModelInfoError::Generalizations)
}
pub(crate) fn count_entries(entries: &IndexMap<String, Box<RawValue>>) -> usize {
    entries
        .keys()
        .filter(|key| !RESERVED_TOP_LEVEL_KEYS.contains(&key.as_str()))
        .count()
}
