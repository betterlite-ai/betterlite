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

use super::call::{AssistantCreateCall, AssistantCreateOutcome, AssistantCreateResult};
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
    pub async fn create_assistants(&self, input: &AssistantCreateInput) -> AssistantCreateOutcome {
        let (provider, flag) = self.selection(input, Entry::Sync)?;
        let call = self.call(input, &provider, Entry::Sync, matches!(flag, Some(true)))?;
        self.answer(input, provider, Entry::Sync, flag, call).await
    }

    pub async fn acreate_assistants(&self, input: &AssistantCreateInput) -> AssistantCreateOutcome {
        let (provider, flag) = self.selection(input, Entry::Async)?;
        let call = self.call(input, &provider, Entry::Async, true)?;
        self.answer(input, provider, Entry::Async, flag, call).await
    }

    async fn answer(
        &self,
        input: &AssistantCreateInput,
        provider: String,
        entry: Entry,
        flag: Option<bool>,
        call: AssistantCreateCall,
    ) -> AssistantCreateOutcome {
        match self.execute(&call, flag).await {
            Ok(Some(assistant)) => Ok(AssistantCreateResult { call, assistant }),
            Ok(None) => Err(error::no_response()),
            Err(raised) => Err(error::raised(
                raised,
                &input.model,
                &provider,
                entry,
                self.redactor,
            )),
        }
    }

    // Every façade failure carries the model, the provider it was called with and the entry raising it.
    fn failure(
        &self,
        input: &AssistantCreateInput,
        provider: &str,
        entry: Entry,
        message: &str,
    ) -> AssistantCreateFailure {
        error::raised(
            Raised::Local(message.to_owned()),
            input.model.as_str(),
            provider,
            entry,
            self.redactor,
        )
    }

    // acreate resolves the model and forces the flag true; the sync façade refuses a non-bool flag.
    fn selection(
        &self,
        input: &AssistantCreateInput,
        entry: Entry,
    ) -> Result<(String, Option<bool>), AssistantCreateFailure> {
        if entry == Entry::Async {
            let provider = projection::provider(input, self.resolution).map_err(|source| {
                self.failure(input, &input.provider, Entry::Async, &source.to_string())
            })?;
            return Ok((provider, Some(true)));
        }
        let flag = match &input.async_create_assistants {
            Some(AsyncFlag::Bool(value)) => Some(*value),
            Some(AsyncFlag::Other(_)) => {
                return Err(self.failure(input, &input.provider, Entry::Sync, INVALID_FLAG));
            }
            None => None,
        };
        Ok((input.provider.clone(), flag))
    }

    fn call(
        &self,
        input: &AssistantCreateInput,
        provider: &str,
        entry: Entry,
        is_async: bool,
    ) -> Result<AssistantCreateCall, AssistantCreateFailure> {
        let kind = match provider {
            "openai" => projection::CallKind::OpenAi,
            "azure" => projection::CallKind::Azure,
            _ => return Err(error::unsupported(provider)),
        };
        let timeout = projection::timeout(input, provider)
            .map_err(|source| self.failure(input, provider, entry, &source.to_string()))?;
        Ok(projection::call(projection::CallRequest {
            kind,
            settings: &self.settings,
            environment: self.environment,
            input,
            timeout,
            is_async,
        }))
    }

    async fn execute(
        &self,
        call: &AssistantCreateCall,
        flag: Option<bool>,
    ) -> Result<Option<AssistantObject>, Raised> {
        match call {
            AssistantCreateCall::OpenAi(openai) => {
                let deadline = projection::deadline(openai.timeout)
                    .map_err(|source| Raised::Local(source.to_string()))?;
                let max_retries = projection::retries(openai.max_retries)
                    .map_err(|source| Raised::Local(source.to_string()))?;
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
                    .map_err(Raised::from)?;
                match reply.reply {
                    DefinitionReply::Assistant(assistant) => Ok(Some(*assistant)),
                    DefinitionReply::Page(_) | DefinitionReply::Deleted(_) => Ok(None),
                }
            }
        }
    }
}
