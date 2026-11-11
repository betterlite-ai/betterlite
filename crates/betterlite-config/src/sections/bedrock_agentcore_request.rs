use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawFields,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawFields,
}

#[derive(Debug, Default, Deserialize)]
struct RawFields {
    forward_multimodal_content: Option<ForwardingValue>,
    #[serde(rename = "runtimeSessionId")]
    runtime_session_id: Option<String>,
    #[serde(rename = "runtimeUserId")]
    runtime_user_id: Option<String>,
    qualifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ForwardingValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Mapping(IndexMap<String, Self>),
}

impl ForwardingValue {
    // LiteLLM reads only 1/true/yes/on as true from a string; every other string is false, including "false"
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Text(text) => {
                let lowered = text.trim().to_lowercase();
                lowered == "1" || lowered == "true" || lowered == "yes" || lowered == "on"
            }
            Self::Null => false,
            Self::Boolean(value) => *value,
            Self::Integer(value) => *value != 0,
            Self::Unsigned(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::List(items) => !items.is_empty(),
            Self::Mapping(fields) => !fields.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub forward_multimodal_content: Option<ForwardingValue>,
    pub runtime_session_id: Option<String>,
    pub runtime_user_id: Option<String>,
    pub qualifier: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut merged = resolve_fields(self.litellm_settings, "litellm_settings", lookup)?;
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            merged.overlay(resolve_fields(deployment.litellm_params, &prefix, lookup)?);
        }
        Ok(merged)
    }
}

impl Settings {
    // deployment litellm_params win over global litellm_settings; the last deployment wins ties
    fn overlay(&mut self, other: Self) {
        if other.forward_multimodal_content.is_some() {
            self.forward_multimodal_content = other.forward_multimodal_content;
        }
        if other.runtime_session_id.is_some() {
            self.runtime_session_id = other.runtime_session_id;
        }
        if other.runtime_user_id.is_some() {
            self.runtime_user_id = other.runtime_user_id;
        }
        if other.qualifier.is_some() {
            self.qualifier = other.qualifier;
        }
    }
}

fn resolve_fields<L>(raw: RawFields, prefix: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let forward_path = format!("{prefix}.forward_multimodal_content");
    let forward = match raw.forward_multimodal_content {
        Some(ForwardingValue::Text(text)) => Some(ForwardingValue::Text(resolve_string(
            text,
            &forward_path,
            lookup,
        )?)),
        Some(other) => Some(other),
        None => None,
    };
    Ok(Settings {
        forward_multimodal_content: forward,
        runtime_session_id: resolve_text(
            raw.runtime_session_id,
            &format!("{prefix}.runtimeSessionId"),
            lookup,
        )?,
        runtime_user_id: resolve_text(
            raw.runtime_user_id,
            &format!("{prefix}.runtimeUserId"),
            lookup,
        )?,
        qualifier: resolve_text(raw.qualifier, &format!("{prefix}.qualifier"), lookup)?,
    })
}

fn resolve_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(None),
        Some(text) => Ok(Some(resolve_string(text, path, lookup)?)),
    }
}

mod tests;
