use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::BatchPage;
use betterlite_openai::batches::sdk::BatchesSdk;
use secrecy::SecretString;
use serde::Deserialize;

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{
    CallableProvider, ClientOutcome, LiteLlmParams, SessionKind, SuppliedClient,
};

use super::error::AzureBatchesError;

// The api_key, api_base and api_version arguments of the handler, without the ones it never forwards
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ClientSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
}

// Which wording the async class check uses; cancel names both async classes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientCheck {
    Lifecycle,
    Cancel,
}

// The SDK client a supplied, cached or constructed outcome is handed back as
pub trait ClientMaterializer {
    type Client: BatchesSdk<Page = BatchPage>;

    // Ok(None) is an SDK that could not be built, for example without any API key
    fn materialize(&self, outcome: &ClientOutcome<'_>) -> Result<Option<Self::Client>, SdkError>;
}

pub struct Connection<'cache, 'data, M: ClientMaterializer> {
    pub settings: ClientSettings,
    // litellm_params: the only place the handler forwards timeout, max_retries and the ad token from
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
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
    pub fn get_openai_client(&mut self) -> Result<ClientOutcome<'data>, AzureBatchesError> {
        let request = GetClientRequest {
            api_key: self.settings.api_key.clone(),
            api_base: self.settings.api_base.clone(),
            api_version: self.settings.api_version.clone(),
            model: None,
            is_async: self.is_async,
            params: self.params.clone(),
            provider: self.provider.clone(),
            supplied: self.supplied.clone(),
            settings: self.endpoints,
            environment: self.environment,
            refresh_enabled: self.refresh_enabled,
            session: self.session,
            sources: self.sources,
        };
        get_azure_openai_client(&request, &mut *self.cache, &mut *self.refresh)
            .map_err(AzureBatchesError::from)
    }

    // LiteLLM checks the missing client first, then the class the operation allows
    pub fn open(mut self, check: ClientCheck) -> Result<M::Client, AzureBatchesError> {
        let outcome = self.get_openai_client()?;
        let client = self
            .materializer
            .materialize(&outcome)?
            .ok_or(AzureBatchesError::ClientMissing)?;
        if self.is_async && client.kind() != ClientKind::Asynchronous {
            return Err(match check {
                ClientCheck::Cancel => AzureBatchesError::CancelNotAsync,
                ClientCheck::Lifecycle => AzureBatchesError::NotAsync,
            });
        }
        // only cancel checks the sync class; the other sync calls hand the client on as it is
        if !self.is_async && check == ClientCheck::Cancel && client.kind() != ClientKind::Sync {
            return Err(AzureBatchesError::NotSync);
        }
        Ok(client)
    }
}
