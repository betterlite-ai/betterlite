use betterlite_config::credentials::CredentialValues;
use serde_json::value::{RawValue, to_raw_value};
use thiserror::Error;

use super::config::{IngestionSetupError, VectorStoreConfig};

pub const CREDENTIAL_NAME_KEY: &str = "litellm_credential_name";

// a stored credential owns these endpoint fields, so a caller value it omits is dropped
pub const STORED_ENDPOINT_KEYS: [&str; 3] =
    ["api_base", "aws_sts_endpoint", "aws_web_identity_token"];

pub trait CredentialLookup: Send + Sync {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialLookupError {
    #[error("credential `{credential_name}` was not found")]
    NotFound { credential_name: String },
}

pub fn load_from_config(
    config: &mut VectorStoreConfig,
    credentials: &dyn CredentialLookup,
) -> Result<(), IngestionSetupError> {
    let Some(raw_name) = config.get(CREDENTIAL_NAME_KEY) else {
        return Ok(());
    };
    let Some(name) = credential_name(raw_name)? else {
        return Ok(());
    };
    let stored = match credentials.credential_values(&name) {
        Ok(stored) => stored,
        Err(CredentialLookupError::NotFound { .. }) => return Ok(()),
    };
    if stored.0.is_empty() {
        return Ok(());
    }
    for (key, value) in &stored.0 {
        let raw_value = to_raw_value(&value.expose_json()).map_err(|source| {
            IngestionSetupError::CredentialValue {
                field: key.clone(),
                source,
            }
        })?;
        config.insert(key.clone(), raw_value);
    }
    for key in STORED_ENDPOINT_KEYS {
        if config.contains_key(key) && !stored.0.contains_key(key) {
            config.remove(key);
        }
    }
    Ok(())
}

fn credential_name(raw_name: &RawValue) -> Result<Option<String>, IngestionSetupError> {
    serde_json::from_str(raw_name.get()).map_err(|_source| IngestionSetupError::InvalidField {
        field: CREDENTIAL_NAME_KEY,
        expected: "a string",
    })
}
