use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::records::{
    AssistantCreate, AssistantDeleted, AssistantObject, AssistantPage,
};
use betterlite_openai::assistants::transport::AssistantsTransport;
use secrecy::SecretString;
use serde::Serialize;

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
    pub environment: &'data (dyn EnvironmentLookup + Sync),
    pub sources: &'data (dyn CredentialSources + Sync),
}

pub struct AssistantsApi<T> {
    pub transport: Arc<T>,
}

// What one facade call asks the SDK client to do.
pub enum DefinitionCall<'call> {
    List,
    Create(&'call AssistantCreate),
    Delete(&'call str),
}

// The SDK object a facade call returns, each one exactly as the SDK built it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum DefinitionReply {
    Page(Box<AssistantPage>),
    Assistant(Box<AssistantObject>),
    Deleted(Box<AssistantDeleted>),
}

// The SDK's reply stays untouched; the kind names which SDK client the call chose, as ClientOutcome does for B531.
#[derive(Debug)]
pub struct AssistantsReply {
    pub reply: DefinitionReply,
    pub kind: ClientKind,
}

impl<T: AssistantsTransport> AssistantsApi<T> {
    // Only an absent supplied client constructs one; a supplied client is returned as-is, whatever its class.
    pub fn azure_client<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        kind: ClientKind,
    ) -> Result<AzureAssistantsClient<'data, T>, DefinitionsError> {
        let Some(client) = supplied else {
            return self.construct(request, kind);
        };
        Ok(client)
    }

    // Serves every definition facade: the flag picks the client kind and the call picks the SDK method.
    pub async fn call<'data>(
        &self,
        request: &ClientRequest<'data>,
        supplied: Option<AzureAssistantsClient<'data, T>>,
        flag: Option<bool>,
        definition: DefinitionCall<'_>,
    ) -> Result<AssistantsReply, DefinitionsError> {
        let client = self.azure_client(request, supplied, requested_kind(flag))?;
        let kind = client.kind;
        let reply = match definition {
            DefinitionCall::List => DefinitionReply::Page(Box::new(client.list().await?)),
            DefinitionCall::Create(data) => {
                DefinitionReply::Assistant(Box::new(client.create(data).await?))
            }
            DefinitionCall::Delete(assistant_id) => {
                DefinitionReply::Deleted(Box::new(client.delete(assistant_id).await?))
            }
        };
        Ok(AssistantsReply { reply, kind })
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
                is_async: kind == ClientKind::Asynchronous,
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

// Only the literal true flag selects the async client; a missing, false or other facade value stays sync.
const fn requested_kind(flag: Option<bool>) -> ClientKind {
    match flag {
        Some(true) => ClientKind::Asynchronous,
        Some(false) | None => ClientKind::Sync,
    }
}
