use std::fmt::{Debug, Formatter, Result as FmtResult};

use betterlite_models::decisions::{Extra, Json};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

mod tests;

// the parts of main.py's embedding aliases that B1301 does not already read
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

// the per-deployment options main.py's embedding branch reads; every other litellm_params key stays unread here
#[derive(Debug, Default, serde::Deserialize)]
struct RawParams {
    vertex_project: Option<Json>,
    vertex_ai_project: Option<Json>,
    vertex_location: Option<Json>,
    vertex_ai_location: Option<Json>,
    vertex_credentials: Option<Json>,
    vertex_ai_credentials: Option<Json>,
    api_base: Option<Json>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub project: Option<String>,
    // boxed so this section adds little to every Config held across an await
    pub credentials: Option<Box<ConfiguredCredentials>>,
    pub api_base: Option<String>,
    // one entry per model_list entry, in file order
    pub deployments: Vec<PinParams>,
}

// the pinned options hold service-account documents, so a diagnostic names the keys and never the values
#[derive(Clone, Default)]
pub struct PinParams(pub Extra);

impl Debug for PinParams {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::new();
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(pin_params(index, deployment.litellm_params, lookup)?);
        }
        Ok(Settings {
            project: read(lookup, &["VERTEX_PROJECT"]),
            credentials: parse_credentials(read(lookup, &["VERTEX_CREDENTIALS"]))?,
            api_base: match self.litellm_settings.api_base {
                Some(text) => Some(resolve_string(text, "litellm_settings.api_base", lookup)?),
                None => read(lookup, &["VERTEXAI_API_BASE", "VERTEX_API_BASE"]),
            },
            deployments,
        })
    }
}

// a text pin may name an environment variable the way every litellm_params value can
fn pin_params(
    index: usize,
    params: RawParams,
    lookup: &dyn EnvironmentLookup,
) -> Result<PinParams, ConfigError> {
    let named = [
        ("vertex_project", params.vertex_project),
        ("vertex_ai_project", params.vertex_ai_project),
        ("vertex_location", params.vertex_location),
        ("vertex_ai_location", params.vertex_ai_location),
        ("vertex_credentials", params.vertex_credentials),
        ("vertex_ai_credentials", params.vertex_ai_credentials),
        ("api_base", params.api_base),
    ];
    let mut pinned = Extra::new();
    for (name, option) in named {
        let Some(value) = option else {
            continue;
        };
        let path = format!("model_list.{index}.litellm_params.{name}");
        let resolved = match value {
            Json::Text(text) => Json::Text(resolve_string(text, &path, lookup)?),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => value,
        };
        pinned.insert(name.to_owned(), resolved);
    }
    Ok(PinParams(pinned))
}

// an empty environment variable reads as unset everywhere LiteLLM reads one
fn read(lookup: &dyn EnvironmentLookup, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| lookup.lookup(name).filter(|text| !text.is_empty()))
}

fn parse_credentials(
    text: Option<String>,
) -> Result<Option<Box<ConfiguredCredentials>>, ConfigError> {
    let Some(credentials) = text else {
        return Ok(None);
    };
    ConfiguredCredentials::from_text(credentials)
        .map(|parsed| Some(Box::new(parsed)))
        .map_err(|_source| ConfigError::InvalidScalar {
            path: "VERTEX_CREDENTIALS".to_owned(),
            value: false,
        })
}
