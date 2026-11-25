use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;

// What the row helpers pass up: LiteLLM raises out of decrypt_model_list_from_db for both.
#[derive(Debug, thiserror::Error)]
pub enum DbRowDecryptError {
    #[error(transparent)]
    Deployment(#[from] InvalidDeployment),
    #[error(transparent)]
    SecretLookup(#[from] SecretLookupError),
}
