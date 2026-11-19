#[path = "vector_store_registry/tests.rs"]
mod tests;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

// one vector_store_registry entry: the name sits at the top level, every store parameter under litellm_params, exactly the two keys LiteLLM_VectorStoreConfig declares
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct RegistryEntry {
    #[serde(default)]
    pub vector_store_name: Option<String>,
    #[serde(default)]
    pub litellm_params: Option<JsonObject>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub stores: Vec<RegistryEntry>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // an absent or null section is no configured registry, an explicit list is used as given
    #[serde(default)]
    pub vector_store_registry: Option<Vec<RegistryEntry>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let entries = self.vector_store_registry.unwrap_or_default();
        let mut stores = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            stores.push(resolve_entry(entry, index, lookup)?);
        }
        Ok(Settings { stores })
    }
}

fn resolve_entry<L>(
    entry: RegistryEntry,
    index: usize,
    lookup: &L,
) -> Result<RegistryEntry, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("vector_store_registry.{index}");
    let vector_store_name = match entry.vector_store_name {
        None => None,
        Some(name) => Some(resolve_string(
            name,
            &format!("{path}.vector_store_name"),
            lookup,
        )?),
    };
    let litellm_params = match entry.litellm_params {
        None => None,
        Some(params) => Some(resolve_object(
            params,
            &format!("{path}.litellm_params"),
            lookup,
        )?),
    };
    Ok(RegistryEntry {
        vector_store_name,
        litellm_params,
    })
}

fn resolve_object<L>(params: JsonObject, path: &str, lookup: &L) -> Result<JsonObject, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = JsonObject::with_capacity(params.len());
    for (name, value) in params {
        let field_path = format!("{path}.{name}");
        resolved.insert(name, resolve_value(value, &field_path, lookup)?);
    }
    Ok(resolved)
}

fn resolve_value<L>(value: Json, path: &str, lookup: &L) -> Result<Json, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => resolve_string(text, path, lookup).map(Json::Text),
        Json::Object(fields) => resolve_object(fields, path, lookup).map(Json::Object),
        Json::List(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for (index, item) in items.into_iter().enumerate() {
                let item_path = format!("{path}.{index}");
                // _resolved_config_value resolves an object inside a list but never a bare string
                let value = match item {
                    Json::Object(fields) => {
                        resolve_object(fields, &item_path, lookup).map(Json::Object)?
                    }
                    other @ (Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_)) => other,
                };
                resolved.push(value);
            }
            Ok(Json::List(resolved))
        }
        other @ (Json::Null | Json::Bool(_) | Json::Number(_)) => Ok(other),
    }
}
