use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_secrets::secret_codec::CipherSettings;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(super) struct KeySource<'input> {
    pub(super) salt_key: Option<&'input str>,
    pub(super) master_key: Option<&'input str>,
    pub(super) general_settings: Option<&'input Value>,
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
