use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::vault::error::VaultResponseError;

#[derive(Debug, Deserialize)]
pub struct VaultSecretReadResponse {
    pub data: Option<VaultSecretDataBlock>,
}

#[derive(Debug, Deserialize)]
pub struct VaultSecretDataBlock {
    pub data: Option<BTreeMap<String, VaultValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum VaultValue {
    Text(String),
    Other(IgnoredAny),
}

impl VaultSecretReadResponse {
    #[must_use]
    pub fn value(&self, data_key: &str) -> Option<&str> {
        let values = self.data.as_ref()?.data.as_ref()?;
        match values.get(data_key)? {
            VaultValue::Text(text) => Some(text),
            VaultValue::Other(_) => None,
        }
    }
}

pub fn vault_secret_read_body(body: &[u8]) -> Result<VaultSecretReadResponse, VaultResponseError> {
    match serde_json::from_slice(body) {
        Ok(parsed) => Ok(parsed),
        Err(_source) => Err(VaultResponseError::InvalidJson),
    }
}
