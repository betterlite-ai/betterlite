use indexmap::IndexMap;
use serde::Deserialize;
use thiserror::Error;

mod tests;

const SANDBOX_TOOLS_PATH: &str = "sandbox_tools";

// the YAML under sandbox_tools is a plain list of dicts in LiteLLM, so an entry has no declared shape
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum SandboxToolValue {
    Null,
    Boolean(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Mapping(IndexMap<String, Self>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxToolConfig {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxToolEntry {
    Configured(SandboxToolConfig),
    NotAnObject,
}

#[derive(Debug, Error)]
pub enum SandboxToolEntryError {
    #[error("{path} must be a list of sandbox tools")]
    ToolsNotList { path: String },
    #[error("{path} must be a string")]
    FieldNotText { path: String },
    #[error("{path} must be a mapping")]
    ParamsNotMapping { path: String },
}

impl SandboxToolEntryError {
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::ToolsNotList { path }
            | Self::FieldNotText { path }
            | Self::ParamsNotMapping { path } => path,
        }
    }
}

pub fn sandbox_tool_entries(
    value: SandboxToolValue,
) -> Result<Vec<SandboxToolEntry>, SandboxToolEntryError> {
    let SandboxToolValue::List(items) = value else {
        return Err(SandboxToolEntryError::ToolsNotList {
            path: SANDBOX_TOOLS_PATH.to_owned(),
        });
    };
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        let path = format!("{SANDBOX_TOOLS_PATH}[{index}]");
        let SandboxToolValue::Mapping(mut fields) = item else {
            entries.push(SandboxToolEntry::NotAnObject);
            continue;
        };
        let params_path = format!("{path}.litellm_params");
        let mut params = match fields.shift_remove("litellm_params") {
            None | Some(SandboxToolValue::Null) => IndexMap::new(),
            Some(SandboxToolValue::Mapping(params)) => params,
            Some(
                SandboxToolValue::Boolean(_)
                | SandboxToolValue::Number(_)
                | SandboxToolValue::Text(_)
                | SandboxToolValue::List(_),
            ) => {
                return Err(SandboxToolEntryError::ParamsNotMapping { path: params_path });
            }
        };
        entries.push(SandboxToolEntry::Configured(SandboxToolConfig {
            name: text_field(&mut fields, "sandbox_tool_name", &path)?,
            provider: text_field(&mut params, "sandbox_provider", &params_path)?,
            api_key: text_field(&mut params, "api_key", &params_path)?,
            api_base: text_field(&mut params, "api_base", &params_path)?,
        }));
    }
    Ok(entries)
}

fn text_field(
    fields: &mut IndexMap<String, SandboxToolValue>,
    name: &str,
    path: &str,
) -> Result<Option<String>, SandboxToolEntryError> {
    match fields.shift_remove(name) {
        None | Some(SandboxToolValue::Null) => Ok(None),
        Some(SandboxToolValue::Text(text)) => Ok(Some(text)),
        Some(
            SandboxToolValue::Boolean(_)
            | SandboxToolValue::Number(_)
            | SandboxToolValue::List(_)
            | SandboxToolValue::Mapping(_),
        ) => Err(SandboxToolEntryError::FieldNotText {
            path: format!("{path}.{name}"),
        }),
    }
}
