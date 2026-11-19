use std::collections::BTreeMap;

use betterlite_config::credentials::CredentialItem;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem as CredentialRow, CredentialItemFields,
};
use betterlite_db::stored_cipher::DbCipher;
use secrecy::SecretString;

use super::encryption::encrypt_credential_values;
use super::error::CredentialError;
use super::store::CredentialStore;

pub struct CredentialUpdate<'update> {
    pub patch: &'update CredentialItem,
    pub new_encryption_key: Option<&'update SecretString>,
    pub actor: Option<&'update str>,
}

pub fn merged_credential(
    cipher: &DbCipher,
    existing: &CredentialRow,
    patch: &CredentialItem,
    new_encryption_key: Option<&SecretString>,
) -> Result<CredentialRow, CredentialError> {
    let encrypted_patch = encrypt_credential_values(cipher, patch, new_encryption_key)?;
    let credential_name = if encrypted_patch.fields.base.credential_name.is_empty() {
        existing.fields.base.credential_name.clone()
    } else {
        encrypted_patch.fields.base.credential_name.clone()
    };
    let mut credential_values = existing.fields.credential_values.clone();
    credential_values
        .0
        .extend(encrypted_patch.fields.credential_values.0);
    let mut credential_info = existing.fields.base.credential_info.clone();
    if !encrypted_patch.fields.base.credential_info.0.is_empty() {
        // LiteLLM keeps the old info only when it carries a literal credential_info key
        if !credential_info.0.contains_key("credential_info") {
            credential_info = Payload(BTreeMap::new());
        }
        credential_info
            .0
            .extend(encrypted_patch.fields.base.credential_info.0);
    }
    Ok(CredentialRow {
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
    existing: &CredentialRow,
    update: CredentialUpdate<'_>,
) -> Result<CredentialRow, CredentialError> {
    let merged = merged_credential(cipher, existing, update.patch, update.new_encryption_key)?;
    store
        .update(&existing.fields.base.credential_name, &merged, update.actor)
        .await?;
    Ok(merged)
}
