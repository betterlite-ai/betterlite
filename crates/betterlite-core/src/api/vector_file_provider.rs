use std::fmt;

use betterlite_models::model_registration::JsonObject;
use serde::{Deserialize, Serialize};

use crate::api::vector_store_registry::store::VectorStoreRegistry;

mod tests;

// the operation kwargs carry api_key, so Debug prints the names and never a value
#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OperationKwargs(pub JsonObject);

impl fmt::Debug for OperationKwargs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OperationKwargs")?;
        f.debug_list().entries(self.0.keys()).finish()
    }
}

// LiteLLM's registry lookup can raise, so the injected seam returns an error and its caller swallows it
pub trait CredentialLookup: Send + Sync {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError>;
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CredentialLookupError {
    #[error("vector store credentials could not be read: {reason}")]
    Unavailable { reason: String },
}

impl CredentialLookup for VectorStoreRegistry {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        Ok(OperationKwargs(
            self.credentials_for_vector_store(vector_store_id),
        ))
    }
}

#[must_use]
pub fn ensure_provider(custom_llm_provider: Option<&str>) -> &str {
    match custom_llm_provider {
        Some("") | None => "openai",
        Some(name) => name,
    }
}

// _prepare_registry_credentials: a missing registry, an empty answer and a raising lookup all leave the kwargs as they were, with no log line
pub fn prepare_registry_credentials<L>(
    vector_store_id: &str,
    kwargs: &mut OperationKwargs,
    registry: Option<&L>,
) where
    L: CredentialLookup + ?Sized,
{
    let Some(registry) = registry else {
        return;
    };
    let Ok(credentials) = registry.lookup_credentials(vector_store_id) else {
        return;
    };
    kwargs.0.extend(credentials.0);
}
