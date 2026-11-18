use std::collections::BTreeMap;

use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretBox};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::tier_definitions::{
    ParamText, RawFlag, TierDefinition, TierEntry, TierModelEntry, TierParam, TierPool,
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
    let is_reference = text.starts_with("os.environ/");
    let value = resolve_string(text, path, lookup)?;
    if is_reference && value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

pub(super) fn definitions(
    raw: Option<Vec<TierDefinition>>,
    prefix: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Vec<TierDefinition>>, ConfigError> {
    let Some(declared) = raw else {
        return Ok(None);
    };
    let mut resolved = Vec::with_capacity(declared.len());
    for (position, definition) in declared.into_iter().enumerate() {
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
    entries: BTreeMap<String, Option<String>>,
    prefix: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<BTreeMap<String, String>, ConfigError> {
    let mut resolved = BTreeMap::new();
    for (tier, label) in entries {
        let Some(text) = label else {
            continue;
        };
        let path = format!("{prefix}.tier_labels.{tier}");
        if let Some(value) = reference(text, &path, lookup)? {
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
    let Some(declared) = raw else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (tier, configured) in declared {
        let Some(pool) = configured else {
            continue;
        };
        let path = format!("{prefix}.tier_models.{tier}");
        let resolved_pool = match pool {
            TierPool::Single(TierEntry::Model(model)) => reference(model, &path, lookup)?
                .map(|name| TierPool::Single(TierEntry::Model(name))),
            TierPool::Single(TierEntry::Detailed(entry)) => Some(TierPool::Single(
                TierEntry::Detailed(detailed(entry, &path, lookup)?),
            )),
            TierPool::Pool(entries) => Some(TierPool::Pool(pool_entries(entries, &path, lookup)?)),
        };
        resolved.extend(resolved_pool.map(|pool_value| (tier, pool_value)));
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
        let entry_path = format!("{path}.{position}");
        // the proxy substitutes mapping values only, so a model name inside a list stays literal
        let TierEntry::Detailed(detailed_entry) = entry else {
            resolved.push(entry);
            continue;
        };
        resolved.push(TierEntry::Detailed(detailed(
            detailed_entry,
            &entry_path,
            lookup,
        )?));
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
    lookup
        .lookup(&name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: name,
            path: path.to_owned(),
        })
}

fn resolve_params(
    params: IndexMap<String, TierParam>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<IndexMap<String, TierParam>, ConfigError> {
    let mut entries = IndexMap::with_capacity(params.len());
    for (key, value) in params {
        let param_path = format!("{path}.{key}");
        entries.extend(resolve_param(value, &param_path, lookup)?.map(|item| (key, item)));
    }
    Ok(entries)
}

fn resolve_param(
    value: TierParam,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<TierParam>, ConfigError> {
    match value {
        TierParam::Text(text) => Ok(
            reference(text.expose_secret().0.clone(), path, lookup)?.map(|resolved_text| {
                TierParam::Text(SecretBox::new(Box::new(ParamText(resolved_text))))
            }),
        ),
        TierParam::Object(inner) => {
            let mut entries = IndexMap::with_capacity(inner.len());
            for (key, item) in inner {
                let member_path = format!("{path}.{key}");
                entries.extend(
                    resolve_param(item, &member_path, lookup)?.map(|resolved| (key, resolved)),
                );
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
        other @ (TierParam::Number(_) | TierParam::Value(_)) => Ok(Some(other)),
    }
}
