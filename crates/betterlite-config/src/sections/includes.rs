use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::settings_rules::JsonValue as Value;
use crate::types::Config;
use serde::de::Deserialize;
use thiserror::Error;

pub type ConfigMap = BTreeMap<String, Value>;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    _marker: Option<()>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings)
    }
}

#[derive(Debug, Error)]
pub enum IncludeError {
    #[error("could not read config include {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid YAML in config include {path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("config include {path} must contain a mapping")]
    Root { path: PathBuf },
    #[error("'include' must be a list of file paths")]
    EntriesNotList,
    #[error("'include' entry at index {index} must be a string path")]
    EntryNotString { index: usize },
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("could not resolve config includes: {source}")]
    Includes {
        #[source]
        source: IncludeError,
    },
    #[error("could not serialize resolved config: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("could not load resolved config: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
}

pub fn resolve_includes(config: ConfigMap, path: &Path) -> Result<ConfigMap, IncludeError> {
    let root = absolute_path(path)?;
    let mut loaded = BTreeSet::from([root.clone()]);
    let mut pending = VecDeque::new();
    queue_includes(&config, &root, &mut pending)?;
    let mut merged = without_include(config);

    while let Some((entry, declared_in)) = pending.pop_front() {
        let location = resolve_include_file_path(&entry, &declared_in, &root);
        if !loaded.insert(location.clone()) {
            continue;
        }
        let included = read_config(&location)?;
        queue_includes(&included, &location, &mut pending)?;
        merged = merge(merged, without_include(included));
    }
    Ok(merged)
}

fn absolute_path(path: &Path) -> Result<PathBuf, IncludeError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|source| IncludeError::Read {
                path: path.to_path_buf(),
                source,
            })
    }
}

fn resolve_include_file_path(entry: &str, declared_in: &Path, root: &Path) -> PathBuf {
    let declared_relative = declared_in
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(entry);
    let root_relative = root.parent().unwrap_or_else(|| Path::new("")).join(entry);
    if root_relative == declared_relative || !root_relative.exists() {
        return declared_relative;
    }
    if !declared_relative.exists() {
        tracing::warn!(
            include = entry,
            declared_in = ?declared_in,
            fallback = ?root_relative,
            "config include was not found next to its declaring file; using the root-relative file"
        );
        return root_relative;
    }
    tracing::warn!(
        include = entry,
        declared_in = ?declared_in,
        declared = ?declared_relative,
        skipped = ?root_relative,
        "config include matched files next to both configs; using the declaring-relative file"
    );
    declared_relative
}

fn read_config(path: &Path) -> Result<ConfigMap, IncludeError> {
    let source = fs::read_to_string(path).map_err(|source| IncludeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = serde_saphyr::with_deserializer_from_str(&source, |deserializer| {
        Deserialize::deserialize(deserializer)
    })
    .map_err(|yaml_error| IncludeError::Yaml {
        path: path.to_path_buf(),
        source: Box::new(yaml_error),
    })?;
    value_to_map(value, path)
}

fn value_to_map(value: Value, path: &Path) -> Result<ConfigMap, IncludeError> {
    match value {
        Value::Object(map) => Ok(map.into_iter().collect()),
        Value::Null => Ok(BTreeMap::new()),
        Value::Array(_) | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            Err(IncludeError::Root {
                path: path.to_path_buf(),
            })
        }
    }
}

fn queue_includes(
    config: &ConfigMap,
    declared_in: &Path,
    pending: &mut VecDeque<(String, PathBuf)>,
) -> Result<(), IncludeError> {
    let Some(value) = config.get("include") else {
        return Ok(());
    };
    let Some(entries) = value.as_array() else {
        return Err(IncludeError::EntriesNotList);
    };
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry_path) = entry.as_str() else {
            return Err(IncludeError::EntryNotString { index });
        };
        pending.push_back((entry_path.to_owned(), declared_in.to_path_buf()));
    }
    Ok(())
}

fn without_include(mut config: ConfigMap) -> ConfigMap {
    config.remove("include");
    config
}

fn merge(mut base: ConfigMap, included: ConfigMap) -> ConfigMap {
    for (key, included_value) in included {
        if let Some(base_value) = base.get_mut(&key)
            && let (Value::Array(base_values), Value::Array(included_values)) =
                (base_value, &included_value)
        {
            base_values.extend(included_values.iter().cloned());
            continue;
        }
        base.insert(key, included_value);
    }
    base
}

pub trait ConfigLoaderFile {
    // pub fn load_file is a trait method because ConfigLoader already has an inherent implementation in loader.rs.
    fn load_file<P>(&self, config_path: P) -> Result<Config, LoadError>
    where
        P: AsRef<Path>;
}

impl<L> ConfigLoaderFile for ConfigLoader<L>
where
    L: EnvironmentLookup,
{
    fn load_file<P>(&self, config_path: P) -> Result<Config, LoadError>
    where
        P: AsRef<Path>,
    {
        let path = config_path.as_ref();
        let config = read_config(path).map_err(|source| LoadError::Includes { source })?;
        let resolved =
            resolve_includes(config, path).map_err(|source| LoadError::Includes { source })?;
        let resolved_source =
            serde_json::to_string(&resolved).map_err(|source| LoadError::Serialize { source })?;
        self.load(&resolved_source)
            .map_err(|config_error| LoadError::Config {
                source: config_error,
            })
    }
}

mod tests;
