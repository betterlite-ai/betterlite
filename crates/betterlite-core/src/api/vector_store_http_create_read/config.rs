use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use super::error::CapabilityError;
use crate::api::vector_store_http_write::config::{OpenAiWriteConfig, WriteConfig};

// validate_create_vector_store plus the BaseVectorStoreConfig members the three handlers call
pub trait CreateReadConfig: Send + Sync {
    fn validate_create(&self) -> Result<(), CapabilityError>;

    fn provider(&self) -> &dyn WriteConfig;
}

// Only the create handler asks for the capability; B527 gives the OpenAI config the base no-op, so this never refuses
pub struct OpenAiCreateReadConfig<'settings> {
    provider: OpenAiWriteConfig<'settings>,
}

impl<'settings> OpenAiCreateReadConfig<'settings> {
    #[must_use]
    pub const fn new(
        settings: &'settings OpenAiCredentialsSettings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
        litellm_api_base: Option<String>,
    ) -> Self {
        Self {
            provider: OpenAiWriteConfig::new(settings, globals, env_api_key, litellm_api_base),
        }
    }
}

impl CreateReadConfig for OpenAiCreateReadConfig<'_> {
    fn validate_create(&self) -> Result<(), CapabilityError> {
        Ok(())
    }

    fn provider(&self) -> &dyn WriteConfig {
        &self.provider
    }
}
