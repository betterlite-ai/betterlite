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

// the same rule every scalar setting follows: an os.environ/ reference resolves once here, and an
// empty variable counts as unset
fn reference(
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

pub(super) fn labels(
    raw: Option<BTreeMap<String, Option<String>>>,
    prefix: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<BTreeMap<String, String>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let mut resolved = BTreeMap::new();
    for (tier, label) in raw {
        let Some(label) = label else {
            continue;
        };
        let path = format!("{prefix}.tier_labels.{tier}");
        if let Some(label) = reference(label, &path, lookup)? {
            resolved.insert(tier, label);
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
        if let Some(pool) = pool_value(pool, &path, lookup)? {
            resolved.insert(tier, pool);
        }
    }
    Ok(Some(resolved))
}

fn pool_value(
    pool: TierPool,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierPool>, ConfigError> {
    match pool {
        TierPool::Single(entry) => Ok(single_entry(entry, path, lookup)?.map(TierPool::Single)),
        TierPool::Pool(entries) => {
            let mut resolved = Vec::with_capacity(entries.len());
            for (position, entry) in entries.into_iter().enumerate() {
                let path = format!("{path}.{position}");
                if let Some(entry) = listed_entry(entry, &path, lookup)? {
                    resolved.push(entry);
                }
            }
            Ok(Some(TierPool::Pool(resolved)))
        }
    }
}

fn single_entry(
    entry: TierEntry,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierEntry>, ConfigError> {
    let resolved = match entry {
        TierEntry::Model(model) => reference(model, path, lookup)?.map(TierEntry::Model),
        TierEntry::Detailed(entry) => detailed(entry, path, lookup)?.map(TierEntry::Detailed),
    };
    Ok(resolved)
}

// the proxy substitutes mapping values only, so a model name inside a list stays literal
fn listed_entry(
    entry: TierEntry,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierEntry>, ConfigError> {
    match entry {
        TierEntry::Model(model) => Ok(Some(TierEntry::Model(model))),
        TierEntry::Detailed(entry) => detailed(entry, path, lookup),
    }
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
    let values = params(values, &format!("{path}.litellm_params"), lookup)?;
    Ok(Some(TierEntry::Detailed(TierModelEntry {
        model_name,
        litellm_params: Some(values),
    })))
}

fn params(
    values: IndexMap<String, Json>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<IndexMap<String, Json>, ConfigError> {
    let mut resolved = IndexMap::with_capacity(values.len());
    for (key, value) in values {
        let path = format!("{path}.{key}");
        if let Some(value) = param_value(value, &path, lookup)? {
            resolved.insert(key, value);
        }
    }
    Ok(resolved)
}

fn param_value(
    value: Json,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Json>, ConfigError> {
    match value {
        Json::Text(text) => Ok(reference(text, path, lookup)?.map(Json::Text)),
        Json::Object(inner) => Ok(Some(Json::Object(params(inner, path, lookup)?))),
        Json::List(items) => Ok(Some(Json::List(params_list(items, path, lookup)?))),
        other => Ok(Some(other)),
    }
}

// a list value recurses into its mapping items only, matching the proxy's walk
fn params_list(
    items: Vec<Json>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Vec<Json>, ConfigError> {
    let mut resolved = Vec::with_capacity(items.len());
    for (position, item) in items.into_iter().enumerate() {
        let path = format!("{path}.{position}");
        let item = match item {
            Json::Object(inner) => Json::Object(params(inner, &path, lookup)?),
            other => other,
        };
        resolved.push(item);
    }
    Ok(resolved)
}
