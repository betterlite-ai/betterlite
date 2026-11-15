use crate::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

mod tests;

const SANDBOX_TOOLS_PATH: &str = "sandbox_tools";

#[derive(Debug, Clone, Default)]
pub struct SandboxToolConfig {
    pub name: Option<String>,
    pub provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl PartialEq for SandboxToolConfig {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.provider == other.provider
            && self.api_key.as_ref().map(ExposeSecret::expose_secret)
                == other.api_key.as_ref().map(ExposeSecret::expose_secret)
            && self.api_base == other.api_base
    }
}

impl Eq for SandboxToolConfig {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxToolEntry {
    Configured(SandboxToolConfig),
    NotAnObject,
}

#[derive(Debug, Error)]
pub enum SandboxToolEntryError {
    #[error("{path} must be a string")]
    FieldNotText { path: String },
    #[error("{path} must be a mapping")]
    ParamsNotMapping { path: String },
}

impl SandboxToolEntryError {
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::FieldNotText { path } | Self::ParamsNotMapping { path } => path,
        }
    }
}

pub fn sandbox_tool_entries(
    tools: Vec<Json>,
) -> Result<Vec<SandboxToolEntry>, SandboxToolEntryError> {
    let mut entries = Vec::with_capacity(tools.len());
    for (index, item) in tools.into_iter().enumerate() {
        let path = format!("{SANDBOX_TOOLS_PATH}[{index}]");
        let Json::Object(mut fields) = item else {
            entries.push(SandboxToolEntry::NotAnObject);
            continue;
        };
        let params_path = format!("{path}.litellm_params");
        let mut params = match fields.shift_remove("litellm_params") {
            None | Some(Json::Null) => IndexMap::new(),
            Some(Json::Object(params)) => params,
            Some(_) => {
                return Err(SandboxToolEntryError::ParamsNotMapping { path: params_path });
            }
        };
        entries.push(SandboxToolEntry::Configured(SandboxToolConfig {
            name: text_field(&mut fields, "sandbox_tool_name", &path)?,
            provider: text_field(&mut params, "sandbox_provider", &params_path)?,
            api_key: text_field(&mut params, "api_key", &params_path)?.map(SecretString::from),
            api_base: text_field(&mut params, "api_base", &params_path)?,
        }));
    }
    Ok(entries)
}

fn text_field(
    fields: &mut IndexMap<String, Json>,
    name: &str,
    path: &str,
) -> Result<Option<String>, SandboxToolEntryError> {
    match fields.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(SandboxToolEntryError::FieldNotText {
            path: format!("{path}.{name}"),
        }),
    }
}
