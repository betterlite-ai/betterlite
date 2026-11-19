use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi as AzureAssistantsApi, ClientRequest, DefinitionCall, DefinitionReply,
};
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::client::ClientOptions;
use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::assistants::records::AssistantObject;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_redact::secret::Redactor;

use super::call::{AssistantCreateCall, AssistantCreateResult};
use super::error::{AssistantCreateFailure, Entry, Raised};
use super::{AssistantCreateInput, AsyncFlag, error, projection};

const INVALID_FLAG: &str =
    "Invalid value passed in for async_create_assistants. Only bool or None allowed";

// create_assistants and acreate_assistants, with every value the two façades read passed in.
pub struct AssistantRecords<'context, 'resolution, Transport> {
    pub settings: resource_sdk_settings::Settings,
    pub azure: azure_endpoints::Settings,
    pub environment: &'context (dyn EnvironmentLookup + Sync),
    pub sources: &'context (dyn CredentialSources + Sync),
    pub resolution: &'resolution ResolutionContext<'context>,
    pub redactor: &'context Redactor,
    pub transport: Arc<Transport>,
}

impl<Transport: AssistantsTransport> AssistantRecords<'_, '_, Transport> {
    pub async fn create_assistants(
        &self,
        input: &AssistantCreateInput,
    ) -> Result<AssistantCreateResult, AssistantCreateFailure> {
        self.run(input, Entry::Sync).await
    }

    pub async fn acreate_assistants(
        &self,
        input: &AssistantCreateInput,
    ) -> Result<AssistantCreateResult, AssistantCreateFailure> {
        self.run(input, Entry::Async).await
    }

    async fn run(
        &self,
        input: &AssistantCreateInput,
        entry: Entry,
    ) -> Result<AssistantCreateResult, AssistantCreateFailure> {
        let provider = self.provider(input, entry)?;
        let flag = self.flag(input, entry, &provider)?;
        let is_async = matches!(flag, Some(true));
        let call = self.call(input, &provider, entry, is_async)?;
        let assistant = match self.execute(&call, flag).await {
            Ok(Some(assistant)) => assistant,
            Ok(None) => return Err(error::no_response()),
            Err(raised) => {
                return Err(error::raised(
                    raised,
                    &input.model,
                    &provider,
                    entry,
                    self.redactor,
                ));
            }
        };
        Ok(AssistantCreateResult { call, assistant })
    }

    // acreate forces the flag true; the sync facade takes the caller's bool and refuses anything else
    fn flag(
        &self,
        input: &AssistantCreateInput,
        entry: Entry,
        provider: &str,
    ) -> Result<Option<bool>, AssistantCreateFailure> {
        match (entry, &input.async_create_assistants) {
            (Entry::Async, _) => Ok(Some(true)),
            (Entry::Sync, Some(AsyncFlag::Bool(value))) => Ok(Some(*value)),
            (Entry::Sync, Some(AsyncFlag::Other(_))) => Err(error::raised(
                Raised::Local(String::from(INVALID_FLAG)),
                &input.model,
                provider,
                entry,
                self.redactor,
            )),
            (Entry::Sync, None) => Ok(None),
        }
    }

    fn call(
        &self,
        input: &AssistantCreateInput,
        provider: &str,
        entry: Entry,
        is_async: bool,
    ) -> Result<AssistantCreateCall, AssistantCreateFailure> {
        let timeout = projection::timeout(input, provider).map_err(|source| {
            error::raised(
                Raised::Local(source.to_string()),
                &input.model,
                provider,
                entry,
                self.redactor,
            )
        })?;
        match provider {
            "openai" => Ok(AssistantCreateCall::OpenAi(Box::new(projection::openai(
                &self.settings,
                input,
                timeout,
                is_async,
            )))),
            "azure" => Ok(AssistantCreateCall::Azure(Box::new(projection::azure(
                &self.settings,
                self.environment,
                input,
                timeout,
                is_async,
            )))),
            _ => Err(error::unsupported(provider)),
        }
    }

    // Only acreate_assistants resolves the model first; the sync façade hands the declared name straight over.
    fn provider(
        &self,
        input: &AssistantCreateInput,
        entry: Entry,
    ) -> Result<String, AssistantCreateFailure> {
        match entry {
            Entry::Sync => Ok(input.provider.clone()),
            Entry::Async => projection::provider(input, self.resolution).map_err(|message| {
                error::raised(
                    Raised::Local(message),
                    &input.model,
                    &input.provider,
                    entry,
                    self.redactor,
                )
            }),
        }
    }

    async fn execute(
        &self,
        call: &AssistantCreateCall,
        flag: Option<bool>,
    ) -> Result<Option<AssistantObject>, Raised> {
        match call {
            AssistantCreateCall::OpenAi(openai) => {
                let deadline = projection::deadline(openai.timeout).map_err(Raised::Local)?;
                let max_retries = projection::retries(openai.max_retries).map_err(Raised::Local)?;
                let api = AssistantsApi {
                    transport: Arc::clone(&self.transport),
                };
                let assistant = api
                    .create_assistants(
                        ClientOptions {
                            api_key: openai.api_key.clone(),
                            api_base: Some(openai.api_base.clone()),
                            timeout: Some(deadline),
                            max_retries,
                            organization: openai.organization.clone(),
                        },
                        &openai.create,
                        None,
                        matches!(flag, Some(true)),
                    )
                    .await
                    .map_err(Raised::Assistants)?;
                Ok(Some(assistant))
            }
            AssistantCreateCall::Azure(azure) => {
                let request = ClientRequest {
                    params: LiteLlmParams {
                        azure_ad_token: azure.litellm_params.azure_ad_token.clone(),
                        auth: azure.litellm_params.auth.clone(),
                        max_retries: azure.litellm_params.max_retries,
                        timeout: azure.litellm_params.timeout,
                    },
                    provider: None,
                    api_key: azure.api_key.clone(),
                    api_base: azure.api_base.clone(),
                    api_version: azure.api_version.clone(),
                    azure_ad_token: azure.azure_ad_token.clone(),
                    timeout: projection::seconds(azure.timeout),
                    max_retries: azure.max_retries,
                    refresh_enabled: false,
                    session: SessionKind::Dedicated,
                    settings: &self.azure,
                    environment: self.environment,
                    sources: self.sources,
                };
                let api = AzureAssistantsApi {
                    transport: Arc::clone(&self.transport),
                };
                let reply = api
                    .call(&request, None, flag, DefinitionCall::Create(&azure.create))
                    .await
                    .map_err(Raised::Definitions)?;
                match reply.reply {
                    DefinitionReply::Assistant(assistant) => Ok(Some(*assistant)),
                    DefinitionReply::Page(_) | DefinitionReply::Deleted(_) => Ok(None),
                }
            }
        }
    }
}
