use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
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

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
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
            .map(|(index, entry)| entry_settings(entry, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(McpToolsSection { entries })
    }
}

fn entry_settings(entry: Json, index: usize) -> Result<McpToolEntry, ConfigError> {
    let Json::Object(fields) = entry else {
        return Ok(McpToolEntry::NotAMapping);
    };
    Ok(McpToolEntry::Tool(McpToolSettings {
        name: optional_text(fields.get("name"), &format!("{KEY}[{index}].name"))?,
        description: optional_text(
            fields.get("description"),
            &format!("{KEY}[{index}].description"),
        )?,
        input_schema: schema(
            fields.get("input_schema"),
            &format!("{KEY}[{index}].input_schema"),
        )?,
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

// null reads as the default empty schema, so a mapping keeps only an object here
fn schema(value: Option<&Json>, path: &str) -> Result<IndexMap<String, Json>, ConfigError> {
    match value {
        None | Some(Json::Null) => Ok(IndexMap::new()),
        Some(Json::Object(fields)) => Ok(fields.clone()),
        Some(other) => Err(scalar_error(path, other)),
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
