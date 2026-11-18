use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::tier_definitions::{
    RawFlag, TierDefinition, TierEntry, TierModelEntry, TierPool,
};

pub(super) fn flag(
    value: Option<RawFlag>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<bool, ConfigError> {
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(enabled)) => Ok(enabled),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}

// an os.environ/ reference resolves once here, and an empty variable counts as unset
pub(super) fn reference(
    text: String,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<String>, ConfigError> {
    let Some(name) = text.strip_prefix("os.environ/").map(str::to_owned) else {
        return Ok(Some(text));
    };
    let Some(value) = lookup.lookup(&name) else {
        return Err(ConfigError::MissingEnvironment {
            variable: name,
            path: path.to_owned(),
        });
    };
    Ok((!value.is_empty()).then_some(value))
}

pub(super) fn definitions(
    raw: Option<Vec<TierDefinition>>,
    prefix: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Vec<TierDefinition>>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut resolved = Vec::with_capacity(raw.len());
    for (position, definition) in raw.into_iter().enumerate() {
        let path = format!("{prefix}.tier_definitions.{position}");
        let name = reference(definition.name, &format!("{path}.name"), lookup)?.unwrap_or_default();
        let description = definition
            .description
            .map(|text| reference(text, &format!("{path}.description"), lookup))
            .transpose()?
            .flatten();
        resolved.push(TierDefinition { name, description });
    }
    Ok(Some(resolved))
}

// a tier-keyed map whose null entries are dropped and whose present values resolve once
pub(super) fn resolve_map<T>(
    entries: BTreeMap<String, Option<T>>,
    prefix: &str,
    key: &str,
    lookup: &dyn EnvironmentLookup,
    resolve: fn(T, &str, &dyn EnvironmentLookup) -> Result<Option<T>, ConfigError>,
) -> Result<BTreeMap<String, T>, ConfigError> {
    let mut resolved = BTreeMap::new();
    for (tier, value) in entries {
        let Some(value) = value else {
            continue;
        };
        let path = format!("{prefix}.{key}.{tier}");
        if let Some(value) = resolve(value, &path, lookup)? {
            resolved.insert(tier, value);
        }
    }
    Ok(resolved)
}

pub(super) fn pools(
    raw: Option<BTreeMap<String, Option<TierPool>>>,
    prefix: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<BTreeMap<String, TierPool>>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (tier, pool) in raw {
        let Some(pool) = pool else {
            continue;
        };
        let path = format!("{prefix}.tier_models.{tier}");
        let pool = match pool {
            TierPool::Single(TierEntry::Model(model)) => reference(model, &path, lookup)?
                .map(|model| TierPool::Single(TierEntry::Model(model))),
            TierPool::Single(TierEntry::Detailed(entry)) => detailed(entry, &path, lookup)?
                .map(|entry| TierPool::Single(TierEntry::Detailed(entry))),
            TierPool::Pool(entries) => Some(TierPool::Pool(pool_entries(entries, &path, lookup)?)),
        };
        resolved.extend(pool.map(|pool| (tier, pool)));
    }
    Ok(Some(resolved))
}

fn pool_entries(
    entries: Vec<TierEntry>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Vec<TierEntry>, ConfigError> {
    let mut resolved = Vec::with_capacity(entries.len());
    for (position, entry) in entries.into_iter().enumerate() {
        let path = format!("{path}.{position}");
        // the proxy substitutes mapping values only, so a model name inside a list stays literal
        let TierEntry::Detailed(entry) = entry else {
            resolved.push(entry);
            continue;
        };
        let entry = detailed(entry, &path, lookup)?;
        resolved.extend(entry.map(TierEntry::Detailed));
    }
    Ok(resolved)
}

fn detailed(
    entry: TierModelEntry,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierEntry>, ConfigError> {
    let Some(model_name) = reference(entry.model_name, &format!("{path}.model_name"), lookup)?
    else {
        return Ok(None);
    };
    let values = entry.litellm_params.unwrap_or_default();
    let resolved = resolve_json(
        Json::Object(values),
        &format!("{path}.litellm_params"),
        lookup,
    )?;
    let Some(Json::Object(values)) = resolved else {
        return Ok(None);
    };
    Ok(Some(TierEntry::Detailed(TierModelEntry {
        model_name,
        litellm_params: Some(values),
    })))
}

// a JSON value resolves its strings, recursing into mapping values and into the mapping items of lists
fn resolve_json(
    value: Json,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Json>, ConfigError> {
    let resolved = match value {
        Json::Text(text) => return Ok(reference(text, path, lookup)?.map(Json::Text)),
        Json::Object(inner) => {
            let mut entries = IndexMap::with_capacity(inner.len());
            for (key, item) in inner {
                let item = resolve_json(item, &format!("{path}.{key}"), lookup)?;
                entries.extend(item.map(|item| (key, item)));
            }
            Json::Object(entries)
        }
        Json::List(items) => {
            let mut entries = Vec::with_capacity(items.len());
            for (position, item) in items.into_iter().enumerate() {
                let target = format!("{path}.{position}");
                let Json::Object(inner) = item else {
                    entries.push(item);
                    continue;
                };
                entries.extend(resolve_json(Json::Object(inner), &target, lookup)?);
            }
            Json::List(entries)
        }
        other => other,
    };
    Ok(Some(resolved))
}
