use std::fs;
use std::io::Write as _;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::sections::settings::JsonValue;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::entropy::Entropy;
use super::error::CommandsError;
use crate::autoroute::config::document::{
    GeneralSettings, GeneratedConfigDocument, master_key_from_config,
};
use crate::settings_backup::secure::secure_create;

pub const MASTER_KEY_BYTES: usize = 32;

// The generated config carries keys this task does not model, so it stays an ordered passthrough
pub type GeneratedConfig = IndexMap<String, JsonValue>;

// LiteLLM reuses the config's persisted key and mints one only when absent, so clients survive reruns
pub fn ensure_master_key(
    config_path: &Path,
    entropy: &mut dyn Entropy,
) -> Result<SecretString, CommandsError> {
    let text = fs::read_to_string(config_path).map_err(|source| CommandsError::ReadConfig {
        path: config_path.to_owned(),
        source,
    })?;
    let parsed: Option<GeneratedConfig> =
        serde_saphyr::from_str(&text).map_err(|source| CommandsError::CorruptConfig {
            path: config_path.to_owned(),
            source: Box::new(source),
        })?;
    // LiteLLM's TypeAdapter(dict) rejects the null document safe_load reads from an empty file
    let Some(document) = parsed else {
        return Err(CommandsError::EmptyConfig {
            path: config_path.to_owned(),
        });
    };
    if let Some(persisted) = persisted_master_key(&document) {
        return Ok(persisted);
    }
    let master_key = mint_master_key(entropy)?;
    write_master_key(config_path, &document, &master_key)?;
    Ok(master_key)
}

fn mint_master_key(entropy: &mut dyn Entropy) -> Result<SecretString, CommandsError> {
    let mut random = [0_u8; MASTER_KEY_BYTES];
    entropy.fill(&mut random)?;
    Ok(SecretString::from(URL_SAFE_NO_PAD.encode(random)))
}

#[must_use]
fn persisted_master_key(document: &GeneratedConfig) -> Option<SecretString> {
    let JsonValue::Object(settings) = document.get("general_settings")? else {
        return None;
    };
    let stored = settings.get("master_key")?.as_str()?;
    let typed = GeneratedConfigDocument {
        model_list: Vec::new(),
        general_settings: Some(GeneralSettings {
            master_key: Some(SecretString::from(stored.to_owned())),
        }),
    };
    master_key_from_config(&typed)
}

fn write_master_key(
    config_path: &Path,
    document: &GeneratedConfig,
    master_key: &SecretString,
) -> Result<(), CommandsError> {
    let mut updated = document.clone();
    let settings = match updated.get("general_settings") {
        Some(JsonValue::Object(existing)) => {
            let mut merged = existing.clone();
            merged.insert(
                "master_key".to_owned(),
                JsonValue::String(master_key.expose_secret().to_owned()),
            );
            JsonValue::Object(merged)
        }
        _ => serde_json::to_value(GeneralSettings {
            master_key: Some(master_key.clone()),
        })
        .map_err(|source| CommandsError::EncodeConfig {
            path: config_path.to_owned(),
            source,
        })?,
    };
    updated.insert("general_settings".to_owned(), settings);
    let yaml =
        serde_saphyr::to_string(&updated).map_err(|source| CommandsError::SerializeConfig {
            path: config_path.to_owned(),
            source: Box::new(source),
        })?;
    let mut file = secure_create(config_path)?;
    file.write_all(yaml.as_bytes())
        .map_err(|source| CommandsError::WriteConfig {
            path: config_path.to_owned(),
            source,
        })
}
