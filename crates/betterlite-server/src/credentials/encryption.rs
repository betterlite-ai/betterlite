use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::CredentialError;

pub fn encrypt_credential_values(
    cipher: &DbCipher,
    credential: &CredentialItem,
    new_encryption_key: Option<&SecretString>,
) -> Result<CredentialItem, CredentialError> {
    let mut encrypted = BTreeMap::new();
    for (key, value) in &credential.fields.credential_values.0 {
        let stored = match value {
            JsonData::String(text) => StoredValue::Text(SecretString::from(text.clone())),
            other => StoredValue::Other(other.clone()),
        };
        let sealed = cipher
            .encrypt_value(stored, new_encryption_key)
            .map_err(|source| CredentialError::Encryption { source })?;
        let value = match sealed {
            StoredValue::Text(text) => JsonData::String(text.expose_secret().to_owned()),
            StoredValue::Other(other) => other,
        };
        encrypted.insert(key.clone(), value);
    }
    Ok(CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: credential.fields.base.credential_name.clone(),
                credential_info: credential.fields.base.credential_info.clone(),
            },
            credential_values: Payload(encrypted),
        },
    })
}
