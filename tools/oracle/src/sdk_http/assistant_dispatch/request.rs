use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::assistant_settings::{Config as AssistantConfig, Raw, Settings};
use betterlite_models::decisions::Json;
use serde::Serialize;

use super::super::error::SdkHttpError;
use super::model::{ConfigProjection, OperationCall};

// the assistant_settings mapping the startup block reads
#[derive(Serialize)]
struct SettingsRoot<'config> {
    assistant_settings: &'config ConfigProjection,
}

// The startup block reads assistants_config from YAML once; a direct one bypasses that block entirely.
pub(super) fn assistant_config(
    call: &OperationCall,
    env: &BTreeMap<String, String>,
) -> Result<Settings, SdkHttpError> {
    let Some(config) = call.config.as_ref() else {
        return Ok(Settings::default());
    };
    if call.startup {
        let value = serde_json::to_value(SettingsRoot {
            assistant_settings: config,
        })
        .map_err(SdkHttpError::Serialize)?;
        let raw = serde_json::from_value::<Raw>(value)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        return raw
            .convert(env)
            .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)));
    }
    Ok(Settings {
        config: Some(AssistantConfig {
            custom_llm_provider: config.custom_llm_provider.clone(),
            litellm_params: config.litellm_params.clone(),
        }),
    })
}

// The proxy's **data: the typed body, its parameters, the explicit provider and the mock origin.
pub(super) fn data(call: &OperationCall, api_base: &str) -> BTreeMap<String, Json> {
    let payload = &call.payload;
    let mut data = payload.params.clone();
    for (name, value) in [
        ("assistant_id", &payload.assistant_id),
        ("model", &payload.model),
        ("name", &payload.name),
        ("role", &payload.role),
        ("instructions", &payload.instructions),
        ("additional_instructions", &payload.additional_instructions),
    ] {
        if let Some(value) = value {
            data.insert(name.to_owned(), Json::Text(value.clone()));
        }
    }
    for (name, value) in [
        ("content", &payload.content),
        ("attachments", &payload.attachments),
        ("messages", &payload.messages),
        ("metadata", &payload.metadata),
        ("tool_resources", &payload.tool_resources),
        ("stream", &payload.stream),
        ("tools", &payload.tools),
    ] {
        if let Some(value) = value {
            data.insert(name.to_owned(), value.clone());
        }
    }
    if let Some(provider) = &call.provider {
        data.insert(
            String::from("custom_llm_provider"),
            Json::Text(provider.clone()),
        );
    }
    data.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    data
}
