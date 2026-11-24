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
        // LiteLLM resolves the whole config before it reads mcp_tools, so the references here go first
        let values = match resolved_json(&config, KEY, lookup)? {
            Json::List(values) => values,
            Json::Null => return Ok(McpToolsSection::default()),
            other @ (Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(scalar_error(KEY, &other));
            }
        };
        let entries = values
            .into_iter()
            .enumerate()
            .map(|(index, entry)| entry_settings(entry, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(McpToolsSection { entries })
    }
}

fn entry_settings(entry: Json, index: usize) -> Result<McpToolEntry, ConfigError> {
    let Json::Object(fields) = entry else {
        return Ok(McpToolEntry::NotAMapping);
    };
    let input_schema = match fields.get("input_schema") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(schema)) => schema.clone(),
        Some(other) => {
            return Err(scalar_error(&format!("{KEY}[{index}].input_schema"), other));
        }
    };
    Ok(McpToolEntry::Tool(McpToolSettings {
        name: optional_text(fields.get("name"), &format!("{KEY}[{index}].name"))?,
        description: optional_text(
            fields.get("description"),
            &format!("{KEY}[{index}].description"),
        )?,
        input_schema,
        handler: optional_text(fields.get("handler"), &format!("{KEY}[{index}].handler"))?,
    }))
}

fn optional_text(value: Option<&Json>, path: &str) -> Result<Option<String>, ConfigError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
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
            for (index, item) in items.iter().enumerate() {
                if let Json::Object(fields) = item {
                    let item_path = format!("{path}[{index}]");
                    resolved.push(Json::Object(resolved_fields(fields, &item_path, lookup)?));
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
