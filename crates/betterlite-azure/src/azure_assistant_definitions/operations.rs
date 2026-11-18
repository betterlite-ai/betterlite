use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::assistants::records::{
    AssistantCreate, AssistantDeleted, AssistantObject, AssistantPage,
};
use betterlite_openai::assistants::transport::AssistantsTransport;
use secrecy::SecretString;

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::initialize::{InitializeRequest, initialize_azure_sdk_client};
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind};

pub struct ClientRequest<'data> {
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    // the facade's own azure_ad_token, timeout and max_retries: accepted for signature parity, never forwarded
    pub azure_ad_token: Option<SecretString>,
    pub timeout: f64,
    pub max_retries: Option<i64>,
    pub refresh_enabled: bool,
    pub session: SessionKind,
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
}

pub struct AssistantsApi<T> {
    pub transport: Arc<T>,
}

// The SDK's reply stays untouched; the kind names which SDK client the call chose, as ClientOutcome does for B531.
#[derive(Debug)]
pub struct AssistantsReply<V> {
    pub reply: V,
    pub kind: ClientKind,
}

impl<T: AssistantsTransport> AssistantsApi<T> {
    pub fn get_azure_client<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
    ) -> Result<AzureAssistantsClient<'data, T>, DefinitionsError> {
        match supplied {
            Some(client) => Ok(client),
            None => self.construct(request, ClientKind::Sync),
        }
    }

    pub fn async_get_azure_client<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
    ) -> Result<AzureAssistantsClient<'data, T>, DefinitionsError> {
        match supplied {
            Some(client) => Ok(client),
            None => self.construct(request, ClientKind::Async),
        }
    }

    pub async fn async_get_assistants<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
    ) -> Result<AssistantsReply<AssistantPage>, DefinitionsError> {
        let client = self.async_get_azure_client(request, supplied)?;
        let reply = client.list().await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    // Only the literal true flag selects the async client; a missing, false or other facade value stays sync.
    pub async fn get_assistants<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        aget_assistants: Option<bool>,
    ) -> Result<AssistantsReply<AssistantPage>, DefinitionsError> {
        if requested_kind(aget_assistants) == ClientKind::Async {
            return self.async_get_assistants(request, supplied).await;
        }
        let client = self.get_azure_client(request, supplied)?;
        let reply = client.list().await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    pub async fn async_create_assistants<'data>(
        &self,
        request: &ClientRequest<'data>,
        data: &AssistantCreate,
        supplied: Option<AzureAssistantsClient<'data, T>>,
    ) -> Result<AssistantsReply<AssistantObject>, DefinitionsError> {
        let client = self.async_get_azure_client(request, supplied)?;
        let reply = client.create(data).await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    // Serves create_assistants and async_create_assistants: the flag only selects the client type.
    pub async fn create_assistants<'data>(
        &self,
        request: &ClientRequest<'data>,
        data: &AssistantCreate,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        async_create_assistants: Option<bool>,
    ) -> Result<AssistantsReply<AssistantObject>, DefinitionsError> {
        if requested_kind(async_create_assistants) == ClientKind::Async {
            return self.async_create_assistants(request, data, supplied).await;
        }
        let client = self.get_azure_client(request, supplied)?;
        let reply = client.create(data).await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    pub async fn async_delete_assistant<'data>(
        &self,
        request: &ClientRequest<'data>,
        assistant_id: &str,
        supplied: Option<AzureAssistantsClient<'data, T>>,
    ) -> Result<AssistantsReply<AssistantDeleted>, DefinitionsError> {
        let client = self.async_get_azure_client(request, supplied)?;
        let reply = client.delete(assistant_id).await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    // Serves delete_assistant and async_delete_assistant: the flag only selects the client type.
    pub async fn delete_assistant<'data>(
        &self,
        request: &ClientRequest<'data>,
        assistant_id: &str,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        async_delete_assistants: Option<bool>,
    ) -> Result<AssistantsReply<AssistantDeleted>, DefinitionsError> {
        if requested_kind(async_delete_assistants) == ClientKind::Async {
            return self
                .async_delete_assistant(request, assistant_id, supplied)
                .await;
        }
        let client = self.get_azure_client(request, supplied)?;
        let reply = client.delete(assistant_id).await?;
        Ok(AssistantsReply {
            reply,
            kind: client.kind,
        })
    }

    fn construct<'data>(
        &self,
        request: &ClientRequest<'data>,
        kind: ClientKind,
    ) -> Result<AzureAssistantsClient<'data, T>, DefinitionsError> {
        let mut refresh_cache = RefreshProviderCache::new(request.sources);
        let inputs = initialize_azure_sdk_client(
            &InitializeRequest {
                params: request.params.clone(),
                provider: request.provider.clone(),
                api_key: request.api_key.clone(),
                api_base: request.api_base.clone(),
                model_name: Some(String::new()),
                api_version: request.api_version.clone(),
                is_async: kind == ClientKind::Async,
                session: request.session,
                refresh_enabled: request.refresh_enabled,
                settings: request.settings,
                environment: request.environment,
                sources: request.sources,
            },
            &mut refresh_cache,
        )?;
        AzureAssistantsClient::new(
            kind,
            inputs,
            request.environment,
            Arc::clone(&self.transport),
        )
    }
}

const fn requested_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Async,
        Some(false) | None => ClientKind::Sync,
    }
}
