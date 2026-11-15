use std::fs;
use std::path::PathBuf;

use secrecy::SecretString;
use serde::Deserialize;

use betterlite_secrets::gateway_key::{GatewayKeyStore, StoredCredential};

#[derive(Deserialize)]
struct TokenFile {
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    key: Option<String>,
}

pub(crate) struct TokenFileStore {
    pub(crate) token_file: PathBuf,
}

impl GatewayKeyStore for TokenFileStore {
    fn stored_credential(&self, _expected_base_url: Option<&str>) -> Option<StoredCredential> {
        let text = fs::read_to_string(&self.token_file).ok()?;
        let record: TokenFile = serde_json::from_str(&text).ok()?;
        Some(StoredCredential {
            base_url: record.base_url,
            key: record.key.map(SecretString::from),
        })
    }
}
