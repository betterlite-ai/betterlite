use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::api::request_model_overrides::error::RequestModelOverridesError;

pub(crate) const MODEL_CONFIG_KEY: &str = "model_config";
pub(crate) const DEFAULT_ENTRY_NAME: &str = "defaultconfig";
const CREDENTIALS_KEY: &str = "litellm_credentials";

// which metadata map a model_config came from, named in the field path of a type error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataSlot {
    Project,
    Team,
}

impl MetadataSlot {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project_metadata",
            Self::Team => "team_metadata",
        }
    }
}

// metadata["model_config"] keyed by model; an empty provider map reads as absent, as LiteLLM's truthiness does
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelConfig {
    entries: IndexMap<String, ModelEntry>,
}

impl ModelConfig {
    pub(crate) fn entry(&self, name: &str) -> Option<&ModelEntry> {
        self.entries
            .get(name)
            .filter(|entry| !entry.providers.is_empty())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ModelEntry {
    providers: IndexMap<String, Option<String>>,
}

impl ModelEntry {
    // _extract_credential_from_entry: the hinted provider wins, then the first provider carrying a name
    pub(crate) fn credential(&self, provider: Option<&str>) -> Option<String> {
        if let Some(Some(credential)) = provider.and_then(|name| self.providers.get(name)) {
            return Some(credential.clone());
        }
        self.providers
            .values()
            .find_map(|credential| credential.clone())
    }
}

// the typed view the credential walk reads; null or an empty map is absent, a wrong type names its path
pub(crate) fn model_config_from_metadata(
    metadata: Option<&JsonObject>,
    slot: MetadataSlot,
) -> Result<Option<ModelConfig>, RequestModelOverridesError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let Some(value) = metadata.get(MODEL_CONFIG_KEY) else {
        return Ok(None);
    };
    let Json::Object(entries) = value else {
        if matches!(value, Json::Null) {
            return Ok(None);
        }
        return Err(RequestModelOverridesError::InvalidField {
            field: format!("{}.{MODEL_CONFIG_KEY}", slot.as_str()),
            expected: "an object",
        });
    };
    if entries.is_empty() {
        return Ok(None);
    }
    let mut config = ModelConfig::default();
    for (model, providers) in entries {
        let path = format!("{}.{MODEL_CONFIG_KEY}.{model}", slot.as_str());
        config
            .entries
            .insert(model.clone(), parse_model_entry(providers, &path)?);
    }
    Ok(Some(config))
}

fn parse_model_entry(
    providers: &Json,
    path: &str,
) -> Result<ModelEntry, RequestModelOverridesError> {
    let Json::Object(providers) = providers else {
        return Err(RequestModelOverridesError::InvalidField {
            field: path.to_owned(),
            expected: "an object",
        });
    };
    let mut provider_map = IndexMap::with_capacity(providers.len());
    for (provider, fields) in providers {
        let provider_path = format!("{path}.{provider}");
        let Json::Object(fields) = fields else {
            return Err(RequestModelOverridesError::InvalidField {
                field: provider_path,
                expected: "an object",
            });
        };
        let credential = credential_name_from(fields, &provider_path)?;
        provider_map.insert(provider.clone(), credential);
    }
    Ok(ModelEntry {
        providers: provider_map,
    })
}

// litellm_credentials: absent and null mean no override, and an empty name is falsy in LiteLLM
fn credential_name_from(
    fields: &JsonObject,
    path: &str,
) -> Result<Option<String>, RequestModelOverridesError> {
    match fields.get(CREDENTIALS_KEY) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(name)) if name.is_empty() => Ok(None),
        Some(Json::Text(name)) => Ok(Some(name.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RequestModelOverridesError::InvalidField {
                field: format!("{path}.{CREDENTIALS_KEY}"),
                expected: "a string",
            })
        }
    }
}
