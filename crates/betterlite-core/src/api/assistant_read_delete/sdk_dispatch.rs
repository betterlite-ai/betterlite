use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_assistant_definitions::client::AzureAssistantsClient;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi as AzureAssistantsApi, ClientRequest, DefinitionCall, DefinitionReply,
};
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::azure_urls::EndpointSelection;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::projection::{AzureClientInputs, HttpClient, SessionKind};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_openai::assistants::client::{ClientKind, ClientOptions, SdkClient};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::assistants::records::ListParams;
use betterlite_openai::assistants::transport::AssistantsTransport;
use secrecy::SecretString;
use thiserror::Error;

use super::call::{AssistantCall, AssistantProviderCall, AzureAssistantCall};
use super::request::{AssistantOperation, SuppliedClient, SuppliedClientKind};
use super::{AssistantReadDeleteDispatch, AssistantReply};

#[derive(Debug, Error)]
pub enum AssistantSdkError {
    #[error(transparent)]
    Assistants(#[from] AssistantsError),
    #[error(transparent)]
    Definitions(#[from] DefinitionsError),
}

// a caller that configures no credential providers: every provider lookup names the missing source
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSources;

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing_source("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing_source("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(missing_source("sdk refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(missing_source("oidc"))
    }
}

fn missing_source(source: &str) -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {source} credential source is configured"),
    }
}

// Runs the facade's call through B519's OpenAI operations or B686's Azure ones against one transport.
pub struct SdkDispatch<T, S> {
    pub transport: Arc<T>,
    pub sources: S,
    pub environment: BTreeMap<String, String>,
    pub azure: AzureSettings,
}

impl<T, S> AssistantReadDeleteDispatch for SdkDispatch<T, S>
where
    T: AssistantsTransport,
    S: CredentialSources + Send + Sync,
{
    type Error = AssistantSdkError;

    fn call(
        &self,
        call: &AssistantCall,
    ) -> impl Future<Output = Result<Option<AssistantReply>, AssistantSdkError>> + Send {
        async move {
            let reply = match &call.provider {
                AssistantProviderCall::OpenAi(options) => self.openai(call, options).await?,
                AssistantProviderCall::Azure(values) => self.azure_call(call, values).await?,
            };
            Ok(Some(reply))
        }
    }
}

impl<T, S> SdkDispatch<T, S>
where
    T: AssistantsTransport,
    S: CredentialSources + Send + Sync,
{
    // The flag only picks B519's client type, and both types send the same request through the transport.
    #[cfg_attr(test, mutants::skip)]
    async fn openai(
        &self,
        call: &AssistantCall,
        options: &ClientOptions,
    ) -> Result<AssistantReply, AssistantSdkError> {
        let api = AssistantsApi {
            transport: Arc::clone(&self.transport),
        };
        let supplied = call
            .supplied
            .as_ref()
            .map(|client| openai_client(&self.transport, client));
        let asynchronous = call.kind == ClientKind::Async;
        let reply = match call.operation {
            AssistantOperation::List => AssistantReply::Page(
                api.get_assistants(
                    options.clone(),
                    supplied,
                    asynchronous,
                    &ListParams::default(),
                )
                .await?,
            ),
            AssistantOperation::Delete => AssistantReply::Deleted(
                api.delete_assistant(
                    options.clone(),
                    call.assistant_id.as_deref().unwrap_or_default(),
                    supplied,
                    asynchronous,
                )
                .await?,
            ),
        };
        Ok(reply)
    }

    // The flag only picks B686's client type, and both types send the same request through the transport.
    #[cfg_attr(test, mutants::skip)]
    async fn azure_call(
        &self,
        call: &AssistantCall,
        values: &AzureAssistantCall,
    ) -> Result<AssistantReply, AssistantSdkError> {
        // Azure keeps an OpenAI SDK client, and that client sends the OpenAI request, not an Azure one
        let openai_client_supplied = call.supplied.as_ref().is_some_and(|client| {
            matches!(
                client.kind,
                SuppliedClientKind::OpenAi | SuppliedClientKind::AsyncOpenAi
            )
        });
        if openai_client_supplied {
            return self.openai(call, &ClientOptions::default()).await;
        }
        let api = AzureAssistantsApi {
            transport: Arc::clone(&self.transport),
        };
        let supplied = match call.supplied.as_ref() {
            Some(client) => Some(self.azure_client(client)),
            None => None,
        };
        let environment: &(dyn EnvironmentLookup + Sync) = &self.environment;
        let sources: &(dyn CredentialSources + Sync) = &self.sources;
        let request = ClientRequest {
            params: values.params.clone(),
            provider: None,
            api_key: values.api_key.clone(),
            api_base: values.api_base.clone(),
            api_version: values.api_version.clone(),
            azure_ad_token: values.azure_ad_token.clone(),
            timeout: values.timeout,
            max_retries: values.max_retries,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
            settings: &self.azure,
            environment,
            sources,
        };
        let definition = match call.operation {
            AssistantOperation::List => DefinitionCall::List,
            AssistantOperation::Delete => {
                DefinitionCall::Delete(call.assistant_id.as_deref().unwrap_or_default())
            }
        };
        let flag = Some(call.kind == ClientKind::Async);
        let reply = api.call(&request, supplied, flag, definition).await?;
        Ok(match reply.reply {
            DefinitionReply::Page(page) => AssistantReply::Page(*page),
            DefinitionReply::Deleted(deleted) => AssistantReply::Deleted(*deleted),
            DefinitionReply::Assistant(assistant) => AssistantReply::Assistant(*assistant),
        })
    }

    // a supplied Azure client keeps its own class, key, endpoint and api_version; a flag flip only picks its transport
    #[cfg_attr(test, mutants::skip)]
    fn azure_client<'call>(
        &'call self,
        client: &'call SuppliedClient,
    ) -> Result<AzureAssistantsClient<'call, T>, DefinitionsError> {
        let kind = client.kind.client_kind();
        let inputs = AzureClientInputs {
            endpoint: EndpointSelection {
                azure_endpoint: client.api_base.clone(),
                base_url: None,
            },
            api_version: client
                .api_version
                .clone()
                .or_else(|| self.environment.lookup("AZURE_API_VERSION"))
                .unwrap_or_else(|| self.azure.azure_default_api_version.clone()),
            api_key: client.api_key.clone(),
            azure_ad_token: None,
            token_provider: None,
            max_retries: 0,
            timeout: None,
            http_client: HttpClient {
                is_async: kind == ClientKind::Async,
                shared: false,
            },
        };
        AzureAssistantsClient::new(kind, inputs, &self.environment, Arc::clone(&self.transport))
    }
}

// a supplied OpenAI client keeps its own key, base and defaults, so the facade's options never reach it
fn openai_client<T: AssistantsTransport>(
    transport: &Arc<T>,
    client: &SuppliedClient,
) -> SdkClient<T> {
    SdkClient {
        kind: client.kind.client_kind(),
        api_key: client.api_key.clone(),
        base_url: client.api_base.clone(),
        timeout: Some(Duration::from_secs(600)),
        max_retries: None,
        organization: None,
        transport: Arc::clone(transport),
    }
}
