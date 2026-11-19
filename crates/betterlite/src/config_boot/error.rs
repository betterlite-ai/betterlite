use std::error::Error;

use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;

// CommonProxyErrors.not_premium_user.value, copied including its line break
const NOT_PREMIUM_USER: &str = "You must be a LiteLLM Enterprise user to use this feature. If you have a license please set `LITELLM_LICENSE` in your env. Get a 7 day trial key here: https://www.litellm.ai/enterprise#trial. \nPricing: https://www.litellm.ai/#pricing";

#[derive(Debug, Error)]
pub enum ConfigBootError {
    #[error("could not resolve an environment_variables reference through the secret lookup")]
    Environment {
        #[source]
        source: SecretLookupError,
    },
    #[error("credential_list is not a list of credential items")]
    CredentialList {
        #[source]
        source: serde_json::Error,
    },
    #[error("config field '{field}' must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("{step} failed")]
    Step {
        step: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("Trying to use `worker_registry`{}", NOT_PREMIUM_USER)]
    WorkerRegistryNeedsPremium,
    #[error("Invalid Key Management System selected")]
    InvalidKeyManagementSystem,
    #[error(
        "CustomSecretManagerException - config_file_path is required to load custom secret manager"
    )]
    CustomManagerConfigPath,
    #[error(
        "CustomSecretManagerException - custom_secret_manager field is required in key_management_settings"
    )]
    CustomManagerField,
}
