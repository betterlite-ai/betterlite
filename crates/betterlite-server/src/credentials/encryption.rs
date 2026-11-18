use std::collections::BTreeMap;

use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem as CredentialRow, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::CredentialError;

// decrypt_value_helper(return_original_value=True): a value that does not decrypt stays as stored
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

// the field name decrypt_value reports when a value does not open
const VALUES_FIELD: &str = "credential_values";

#[derive(Clone, Copy)]
enum Direction {
    Seal,
    Open,
}

struct ValuesCodec<'cipher> {
    cipher: &'cipher DbCipher,
    new_encryption_key: Option<&'cipher SecretString>,
    direction: Direction,
    failure: Option<CredentialError>,
}

impl ValuesCodec<'_> {
    fn walk(&mut self, value: &JsonData) -> JsonData {
        match value {
            JsonData::String(text) => self.leaf(text),
            JsonData::Object(fields) => {
                let mut walked = BTreeMap::new();
                for (name, child) in fields {
                    walked.insert(name.clone(), self.walk(child));
                }
                JsonData::Object(walked)
            }
            JsonData::Array(items) => {
                let mut walked = Vec::with_capacity(items.len());
                for item in items {
                    walked.push(self.walk(item));
                }
                JsonData::Array(walked)
            }
            leaf @ (JsonData::Null | JsonData::Bool(_) | JsonData::Number(_)) => leaf.clone(),
        }
    }

    fn leaf(&mut self, text: &str) -> JsonData {
        if let Direction::Open = self.direction {
            let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
            return match self
                .cipher
                .decrypt_value(stored, VALUES_FIELD, KEEP_ORIGINAL)
            {
                Some(StoredValue::Text(plain)) => {
                    JsonData::String(plain.expose_secret().to_owned())
                }
                Some(StoredValue::Other(())) | None => JsonData::String(text.to_owned()),
            };
        }
        let stored = StoredValue::Text(SecretString::from(text.to_owned()));
        match self.cipher.encrypt_value(stored, self.new_encryption_key) {
            Ok(StoredValue::Text(sealed)) => JsonData::String(sealed.expose_secret().to_owned()),
            Ok(StoredValue::Other(original)) => original,
            Err(source) => {
                self.failure = Some(CredentialError::Encryption { source });
                JsonData::String(text.to_owned())
            }
        }
    }
}

pub fn encrypt_credential_values(
    cipher: &DbCipher,
    credential: &CredentialItem,
    new_encryption_key: Option<&SecretString>,
) -> Result<CredentialRow, CredentialError> {
    let mut codec = ValuesCodec {
        cipher,
        new_encryption_key,
        direction: Direction::Seal,
        failure: None,
    };
    let mut encrypted = BTreeMap::new();
    for (key, value) in &credential.credential_values.0 {
        let plain = value.expose_json();
        encrypted.insert(key.clone(), codec.walk(&plain));
    }
    let row = CredentialRow {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: credential.credential_name.clone(),
                credential_info: credential.credential_info.clone(),
            },
            credential_values: Payload(encrypted),
        },
    };
    match codec.failure {
        Some(error) => Err(error),
        None => Ok(row),
    }
}

#[must_use]
pub fn decrypt_credential_values(cipher: &DbCipher, credential: &CredentialRow) -> CredentialItem {
    let mut codec = ValuesCodec {
        cipher,
        new_encryption_key: None,
        direction: Direction::Open,
        failure: None,
    };
    let mut opened = BTreeMap::new();
    for (key, value) in &credential.fields.credential_values.0 {
        opened.insert(key.clone(), codec.walk(value));
    }
    CredentialItem {
        credential_name: credential.fields.base.credential_name.clone(),
        credential_info: credential.fields.base.credential_info.clone(),
        credential_values: CredentialValues::from(Payload(opened)),
    }
}
