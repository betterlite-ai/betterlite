use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::credentials::registry::CredentialRegistry;
use crate::credentials::store::CredentialStore;

use super::decrypted_text;
use super::error::ModelFieldError;

const CREDENTIAL_FIELD: &str = "litellm_credential_name";

// where a named credential can live: the loaded list first, then the credentials table
pub struct CredentialSources<'sources> {
    pub cipher: &'sources DbCipher,
    pub registry: &'sources CredentialRegistry,
    pub store: &'sources dyn CredentialStore,
}

#[must_use]
pub fn stored_credential_name(cipher: &DbCipher, existing: Option<&JsonObject>) -> Option<String> {
    match existing.and_then(|params| params.get(CREDENTIAL_FIELD)) {
        Some(Json::Text(stored)) => Some(decrypted_text(cipher, stored, CREDENTIAL_FIELD)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// an omitted name leaves the credential alone and a null detaches it, so only a sent name is looked up
pub async fn check_credential_name(
    incoming: Option<&JsonObject>,
    existing: Option<&JsonObject>,
    sources: &CredentialSources<'_>,
) -> Result<(), ModelFieldError> {
    let Some(Json::Text(credential_name)) =
        incoming.and_then(|params| params.get(CREDENTIAL_FIELD))
    else {
        return Ok(());
    };
    if credential_name.is_empty() {
        return Err(ModelFieldError::EmptyCredentialName);
    }
    // a model keeps the credential it already holds even after that credential is gone
    if stored_credential_name(sources.cipher, existing).as_deref() == Some(credential_name.as_str())
    {
        return Ok(());
    }
    if sources.registry.find_credential(credential_name).is_some() {
        return Ok(());
    }
    let found = sources
        .store
        .find_by_name(credential_name)
        .await
        .map_err(|source| ModelFieldError::CredentialLookup { source })?;
    if found.is_some() {
        return Ok(());
    }
    Err(ModelFieldError::CredentialNotFound {
        credential_name: credential_name.clone(),
    })
}
