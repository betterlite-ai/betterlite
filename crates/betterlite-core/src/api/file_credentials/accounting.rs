use std::error::Error;

use betterlite_models::controls::call_params::{TRUSTED_CREDENTIALS_KEY, TrustedCredentials};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

use crate::api::file_credentials::catalog::CredentialCatalog;
use crate::api::model_credentials::trusted_snapshot;

#[derive(Debug, Error)]
pub enum AccountingError<SourceError>
where
    SourceError: Error + 'static,
{
    #[error("the credential catalog failed to resolve model `{model_id}`")]
    Catalog {
        model_id: String,
        #[source]
        source: SourceError,
    },
    // LiteLLM's dict spread raises a TypeError here; the field is named instead
    #[error("litellm_metadata must be an object")]
    MetadataNotObject,
}

// add_internal_model_credentials: the deployment's immutable server-side snapshot, which the Bedrock
// file config reads its bucket from. Every failure leaves data alone, so an old batch stays retrievable.
pub fn add_internal_model_credentials<Catalog>(
    data: &mut JsonObject,
    catalog: Option<&Catalog>,
    model_id: Option<&str>,
) -> Option<TrustedCredentials>
where
    Catalog: CredentialCatalog,
{
    let model_id = model_id?;
    let catalog = catalog?;
    let Ok(Some(credentials)) = catalog.credentials_with_provider(model_id, None) else {
        return None;
    };
    // a snapshot replaces any caller-supplied value under the internal key, as LiteLLM overwrites it
    data.shift_remove(TRUSTED_CREDENTIALS_KEY);
    Some(trusted_snapshot(&credentials))
}

// add_deployment_model_info: stamps the resolved model_info into litellm_metadata, the way the router
// stamps a routed call, so a direct batch call is priced by its deployment id.
pub fn add_deployment_model_info<Catalog>(
    data: &mut JsonObject,
    catalog: Option<&Catalog>,
    model_id: &str,
) -> Result<(), AccountingError<Catalog::Error>>
where
    Catalog: CredentialCatalog,
{
    let Some(catalog) = catalog else {
        return Ok(());
    };
    let Some(model_info) =
        catalog
            .resolved_model_info(model_id)
            .map_err(|source| AccountingError::Catalog {
                model_id: model_id.to_owned(),
                source,
            })?
    else {
        return Ok(());
    };
    let mut metadata = match data.get("litellm_metadata") {
        None | Some(Json::Null) => JsonObject::new(),
        Some(Json::Object(existing)) => existing.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AccountingError::MetadataNotObject);
        }
    };
    metadata.insert("model_info".to_owned(), Json::Object(model_info));
    data.insert("litellm_metadata".to_owned(), Json::Object(metadata));
    Ok(())
}
