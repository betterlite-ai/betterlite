use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::tier_definitions::{
    RawFlag, TierDefinition, TierEntry, TierModelEntry, TierParam, TierParams, TierPool,
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
            TierPool::Single(TierEntry::Detailed(entry)) => Some(TierPool::Single(
                TierEntry::Detailed(detailed(entry, &path, lookup)?),
            )),
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
        resolved.push(TierEntry::Detailed(detailed(entry, &path, lookup)?));
    }
    Ok(resolved)
}

fn detailed(
    entry: TierModelEntry,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<TierModelEntry, ConfigError> {
    let model_name = required_model_name(entry.model_name, &format!("{path}.model_name"), lookup)?;
    let litellm_params = resolve_params(
        entry.litellm_params,
        &format!("{path}.litellm_params"),
        lookup,
    )?;
    Ok(TierModelEntry {
        model_name,
        litellm_params,
    })
}

fn required_model_name(
    text: String,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<String, ConfigError> {
    let Some(name) = text.strip_prefix("os.environ/").map(str::to_owned) else {
        return Ok(text);
    };
    let Some(value) = lookup.lookup(&name).filter(|value| !value.is_empty()) else {
        return Err(ConfigError::MissingEnvironment {
            variable: name,
            path: path.to_owned(),
        });
    };
    Ok(value)
}

fn resolve_params(
    params: TierParams,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<TierParams, ConfigError> {
    let mut entries = IndexMap::with_capacity(params.len());
    for (key, value) in params.0 {
        let path = format!("{path}.{key}");
        entries.extend(resolve_param(value, &path, lookup)?.map(|item| (key, item)));
    }
    Ok(TierParams(entries))
}

fn resolve_param(
    value: TierParam,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierParam>, ConfigError> {
    match value {
        TierParam::Text(text) => Ok(reference(text.expose_secret().to_owned(), path, lookup)?
            .map(|text| TierParam::Text(SecretString::from(text)))),
        TierParam::Object(inner) => {
            let mut entries = IndexMap::with_capacity(inner.len());
            for (key, item) in inner {
                let path = format!("{path}.{key}");
                entries.extend(resolve_param(item, &path, lookup)?.map(|item| (key, item)));
            }
            Ok(Some(TierParam::Object(entries)))
        }
        TierParam::List(items) => {
            let mut entries = Vec::with_capacity(items.len());
            for (position, item) in items.into_iter().enumerate() {
                let TierParam::Object(inner) = item else {
                    entries.push(item);
                    continue;
                };
                let target = format!("{path}.{position}");
                entries.extend(resolve_param(TierParam::Object(inner), &target, lookup)?);
            }
            Ok(Some(TierParam::List(entries)))
        }
        other => Ok(Some(other)),
    }
}
