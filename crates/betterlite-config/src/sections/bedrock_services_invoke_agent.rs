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
    #[serde(rename = "sessionID")]
    session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub session_id: Option<String>,
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
        if other.session_id.is_some() {
            self.session_id = other.session_id;
        }
    }
}

fn resolve_fields<L>(raw: RawFields, prefix: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let session_id = match raw.session_id {
        None => None,
        Some(text) => Some(resolve_string(
            text,
            &format!("{prefix}.sessionID"),
            lookup,
        )?),
    };
    Ok(Settings { session_id })
}

mod tests;
