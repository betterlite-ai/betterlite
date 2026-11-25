use std::collections::BTreeMap;

use betterlite_config::credentials::{CredentialItem, CredentialValue, CredentialValues};
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

// decrypt_value_helper's default mode; LiteLLM's `or v` keeps the stored value when decryption gives nothing.
const DROP_UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Error,
    value: FailureValue::Nothing,
};

// decrypt_credentials: a shallow pass over the credential's values, each decrypted or left as stored.
#[must_use]
pub fn decrypt_credential(item: &CredentialItem, cipher: &DbCipher) -> CredentialItem {
    let mut values = BTreeMap::new();
    for (key, value) in &item.credential_values.0 {
        let stored = value.expose_json();
        let opened = match &stored {
            JsonData::String(text) => {
                let secret = StoredValue::<JsonData>::Text(SecretString::from(text.clone()));
                match cipher.decrypt_value(secret, key, DROP_UNREADABLE) {
                    Some(StoredValue::Text(opened)) => {
                        JsonData::String(opened.expose_secret().to_owned())
                    }
                    Some(StoredValue::Other(_)) | None => stored.clone(),
                }
            }
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => stored.clone(),
        };
        values.insert(key.clone(), CredentialValue::from(opened));
    }
    CredentialItem {
        credential_name: item.credential_name.clone(),
        credential_info: item.credential_info.clone(),
        credential_values: CredentialValues(values),
    }
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - LiteLLM's load_credential_list is one config read whose values are already CredentialItems
pub fn credential_list(config: &Config) -> Vec<CredentialItem> {
    config.sections.credential_list.credentials.clone()
}
