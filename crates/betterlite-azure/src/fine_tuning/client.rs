use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::fine_tuning::client::{BuildError, ClientSettings, JobsApi};

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{ClientOutcome, LiteLlmParams, SessionKind};

use super::error::AzureFineTuneError;

pub trait ClientMaterializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError>;
}

pub struct Connection<'cache, 'data, M: ClientMaterializer> {
    pub settings: ClientSettings,
    // LiteLLM's Azure get_openai_client drops every supplied OpenAI or AsyncOpenAI client, Azure subclasses included
    pub client: Option<Box<dyn JobsApi>>,
    pub is_async: bool,
    pub endpoints: &'data EndpointSettings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub session: SessionKind,
    pub refresh_enabled: bool,
    pub cache: &'cache mut dyn ClientCache,
    pub refresh: &'cache mut RefreshProviderCache<'data>,
    pub materializer: &'cache M,
}

impl<'data, M: ClientMaterializer> Connection<'_, 'data, M> {
    pub fn get_openai_client(&mut self) -> Result<ClientOutcome<'data>, AzureFineTuneError> {
        drop(self.client.take());
        let request = GetClientRequest {
            api_key: self.settings.api_key.clone(),
            api_base: self.settings.api_base.clone(),
            api_version: self.settings.api_version.clone(),
            model: None,
            is_async: self.is_async,
            // the public methods pass no litellm_params, so timeout, max_retries and organization never reach the client
            params: LiteLlmParams::default(),
            provider: None,
            supplied: None,
            settings: self.endpoints,
            environment: self.environment,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            sources: self.sources,
        };
        get_azure_openai_client(&request, &mut *self.cache, &mut *self.refresh)
            .map_err(AzureFineTuneError::from)
    }

    pub fn open(mut self) -> Result<Box<dyn JobsApi>, AzureFineTuneError> {
        let is_async = self.is_async;
        let outcome = self.get_openai_client()?;
        let client = self
            .materializer
            .materialize(&outcome)?
            .ok_or(AzureFineTuneError::ClientMissing)?;
        if is_async && !client.is_async() {
            return Err(AzureFineTuneError::NotAsyncClient);
        }
        Ok(client)
    }
}
