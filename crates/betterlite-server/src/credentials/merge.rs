use std::collections::BTreeMap;

use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use secrecy::SecretString;

use super::encryption::encrypt_credential_values;
use super::error::CredentialError;
use super::store::CredentialStore;

pub struct CredentialUpdate<'a> {
    pub patch: &'a CredentialItem,
    pub new_encryption_key: Option<&'a SecretString>,
    pub actor: Option<&'a str>,
}

pub fn merged_credential(
    cipher: &DbCipher,
    existing: &CredentialItem,
    patch: &CredentialItem,
    new_encryption_key: Option<&SecretString>,
) -> Result<CredentialItem, CredentialError> {
    let patch = encrypt_credential_values(cipher, patch, new_encryption_key)?;
    let credential_name = if patch.fields.base.credential_name.is_empty() {
        existing.fields.base.credential_name.clone()
    } else {
        patch.fields.base.credential_name.clone()
    };
    let mut credential_values = existing.fields.credential_values.clone();
    if !patch.fields.credential_values.0.is_empty() {
        credential_values.0.extend(patch.fields.credential_values.0);
    }
    let mut credential_info = existing.fields.base.credential_info.clone();
    if !patch.fields.base.credential_info.0.is_empty() {
        // LiteLLM keeps the old info only when it carries a literal credential_info key
        if !credential_info.0.contains_key("credential_info") {
            credential_info = Payload(BTreeMap::new());
        }
        credential_info
            .0
            .extend(patch.fields.base.credential_info.0);
    }
    Ok(CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name,
                credential_info,
            },
            credential_values,
        },
    })
}

pub async fn update_db_credential(
    store: &dyn CredentialStore,
    cipher: &DbCipher,
    existing: &CredentialItem,
    update: CredentialUpdate<'_>,
) -> Result<CredentialItem, CredentialError> {
    let merged = merged_credential(cipher, existing, update.patch, update.new_encryption_key)?;
    store
        .update(&existing.fields.base.credential_name, &merged, update.actor)
        .await?;
    Ok(merged)
}
