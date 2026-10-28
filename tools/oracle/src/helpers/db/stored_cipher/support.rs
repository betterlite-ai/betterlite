use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::loader::ConfigLoader;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{CipherSettings, NonceSource};
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(super) struct FixedNonces(pub(super) Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

pub(super) struct LogRecorder<'step> {
    pub(super) field: &'step str,
    pub(super) records: Mutex<Vec<Value>>,
}

impl SecretDiagnostics for LogRecorder<'_> {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        let message = event.message();
        let names_field = !self.field.is_empty() && message.contains(self.field);
        let salt_advice =
            message.contains("https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key");
        let level_name = match level {
            Level::Debug => "debug",
            Level::Error => "error",
        };
        if let Ok(mut records) = self.records.lock() {
            records.push(json!({"level": level_name, "names_field": names_field, "salt_advice": salt_advice}));
        }
    }
}

pub(super) struct KeySource<'input> {
    pub(super) salt_key: Option<&'input str>,
    pub(super) master_key: Option<&'input str>,
    pub(super) general_settings: Option<&'input Value>,
}

pub(super) fn key_source<'input>(
    salt_key: Option<&'input String>,
    master_key: Option<&'input String>,
    general_settings: Option<&'input Value>,
) -> KeySource<'input> {
    let salt = salt_key.map(String::as_str);
    KeySource {
        salt_key: salt,
        master_key: master_key.map(String::as_str),
        general_settings,
    }
}

fn load_cipher_settings(source: &KeySource<'_>) -> Option<CipherSettings> {
    let mut general = match source.general_settings {
        Some(Value::Object(settings)) => settings.clone(),
        Some(_) | None => serde_json::Map::new(),
    };
    if let Some(key) = source.master_key {
        general.insert("master_key".to_owned(), Value::String(key.to_owned()));
    }
    let lookup: BTreeMap<String, String> = source
        .salt_key
        .iter()
        .map(|salt| ("LITELLM_SALT_KEY".to_owned(), (*salt).to_owned()))
        .collect();
    let settings_source = json!({"general_settings": general}).to_string();
    let config = ConfigLoader { lookup }.load(&settings_source).ok()?;
    Some(config.general_settings.cipher)
}

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    let error = HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}

pub(super) fn resolve(
    source: &KeySource<'_>,
    nonce_hex: &str,
) -> Result<(CipherSettings, Vec<u8>), Box<HelperError>> {
    let settings =
        load_cipher_settings(source).ok_or_else(|| invalid("the config does not load"))?;
    let nonce = hex::decode(nonce_hex).map_err(|_error| invalid("nonce_hex is not hex"))?;
    Ok((settings, nonce))
}
