use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_config::loader::ConfigLoader;
use betterlite_mcp::session_credentials;
use betterlite_mcp::types::SessionSigningKeys;
use secrecy::SecretString;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SigningConfigFields {
    master_key: String,
    #[serde(default)]
    general_settings: Option<Value>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

pub(crate) fn run_signing_config(fields: &SigningConfigFields) -> Value {
    let mut general = serde_json::Map::new();
    if let Some(settings) = &fields.general_settings {
        general.insert("mcp_session_token_signing".to_owned(), settings.clone());
    }
    let source = json!({ "general_settings": general }).to_string();
    let loaded = ConfigLoader {
        lookup: fields.environment.clone(),
    }
    .load(&source);
    let Ok(config) = loaded else {
        return json!({ "tag": "invalid" });
    };
    let settings = config.sections.mcp_bearer_credentials;
    let resolved = session_credentials::resolve_session_signing_keys(
        &SecretString::from(fields.master_key.as_str()),
        &settings,
    );
    match resolved {
        Ok(SessionSigningKeys::Symmetric(_)) => json!({ "tag": "symmetric" }),
        Ok(SessionSigningKeys::Asymmetric(asymmetric)) => match asymmetric.kid_and_public_key_pem()
        {
            Ok((kid, public_key_pem)) => {
                json!({ "tag": "asymmetric", "kid": kid, "public_key_pem": public_key_pem })
            }
            Err(_error) => json!({ "tag": "invalid" }),
        },
        Err(_error) => json!({ "tag": "invalid" }),
    }
}
