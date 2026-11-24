use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

pub const KEY: &str = "mcp_tools";

// the top-level key is read whole so a non-mapping entry can be kept for the loader to reject
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    mcp_tools: Option<Box<Json>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpToolsSection {
    pub entries: Vec<McpToolEntry>,
}

// the sections! macro names every section's settings type Settings
pub type Settings = McpToolsSection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpToolEntry {
    Tool(McpToolSettings),
    NotAMapping,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpToolSettings {
    pub name: Option<String>,
    pub description: Option<String>,
    pub input_schema: IndexMap<String, Json>,
    pub handler: Option<String>,
}

impl Section for Raw {
    type Settings = McpToolsSection;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(config) = self.mcp_tools else {
            return Ok(McpToolsSection::default());
        };
        let values = match *config {
            Json::List(values) => values,
            Json::Null => return Ok(McpToolsSection::default()),
            other @ (Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(scalar_error(KEY, &other));
            }
        };
        let entries = values
            .into_iter()
            .enumerate()
            .map(|(index, entry)| entry_settings(entry, index, lookup))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(McpToolsSection { entries })
    }
}

fn entry_settings<L>(entry: Json, index: usize, lookup: &L) -> Result<McpToolEntry, ConfigError>
where
    L: EnvironmentLookup,
{
    let Json::Object(fields) = entry else {
        return Ok(McpToolEntry::NotAMapping);
    };
    Ok(McpToolEntry::Tool(McpToolSettings {
        name: optional_text(fields.get("name"), &format!("{KEY}[{index}].name"), lookup)?,
        description: optional_text(
            fields.get("description"),
            &format!("{KEY}[{index}].description"),
            lookup,
        )?,
        input_schema: schema(
            fields.get("input_schema"),
            &format!("{KEY}[{index}].input_schema"),
            lookup,
        )?,
        handler: optional_text(
            fields.get("handler"),
            &format!("{KEY}[{index}].handler"),
            lookup,
        )?,
    }))
}

fn optional_text<L>(
    value: Option<&Json>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(resolve_string(text.clone(), path, lookup)?)),
        Some(other) => Err(scalar_error(path, other)),
    }
}

// null reads as the default empty schema, so a mapping keeps only an object here
fn schema<L>(
    value: Option<&Json>,
    path: &str,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None | Some(Json::Null) => Ok(IndexMap::new()),
        Some(Json::Object(fields)) => resolved_fields(fields, path, lookup),
        Some(other) => Err(scalar_error(path, other)),
    }
}

fn resolved_fields<L>(
    fields: &IndexMap<String, Json>,
    path: &str,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = IndexMap::with_capacity(fields.len());
    for (key, value) in fields {
        resolved.insert(
            key.clone(),
            resolved_json(value, &format!("{path}.{key}"), lookup)?,
        );
    }
    Ok(resolved)
}

// LiteLLM recurses into objects and the dict items of a list, and leaves every other list item as it is
fn resolved_json<L>(value: &Json, path: &str, lookup: &L) -> Result<Json, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => Ok(Json::Text(resolve_string(text.clone(), path, lookup)?)),
        Json::Object(fields) => Ok(Json::Object(resolved_fields(fields, path, lookup)?)),
        Json::List(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                if let Json::Object(fields) = item {
                    resolved.push(Json::Object(resolved_fields(fields, path, lookup)?));
                } else {
                    resolved.push(item.clone());
                }
            }
            Ok(Json::List(resolved))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => Ok(value.clone()),
    }
}

fn scalar_error(path: &str, value: &Json) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: matches!(value, Json::Bool(true)),
    }
}

#[derive(Debug, Error)]
pub enum McpToolsError {
    #[error("invalid YAML: {source}")]
    Yaml {
        #[source]
        source: serde_saphyr::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
}

pub fn decode_mcp_tools(bytes: &[u8]) -> Result<McpToolsSection, McpToolsError> {
    let raw: Raw =
        serde_saphyr::from_slice(bytes).map_err(|source| McpToolsError::Yaml { source })?;
    Section::convert(raw, &BTreeMap::<String, String>::new()).map_err(McpToolsError::Config)
}

mod tests;
