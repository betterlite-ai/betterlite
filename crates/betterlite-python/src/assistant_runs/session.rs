use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_core::api::assistant_messages::error::MessageFailure;
use betterlite_core::api::assistant_messages::provider::MessageContext;
use betterlite_core::api::assistant_messages::service::{
    MessageRequest, Operation, Reply, message_call,
};
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_core::api::assistant_run_sdk::provider::ProviderRuns;
use betterlite_core::api::assistant_run_sdk::service::AssistantRunSdk;
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::thread_runs::stream::RunStream;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::convert::{self, MessageCall, ModuleSettings};
use super::error::CallError;
use crate::completion::setup::outbound_client;

mod tests;

// The lazy manager core builds for a streaming run; the native stream opens it on its first event.
pub(crate) type ProviderRunStream = RunStream<ProxiedClient>;

// Everything one Python session owns: the environment snapshot, the HTTP client and the assistant settings.
pub(crate) struct Session {
    environment: BTreeMap<String, String>,
    settings: ResourceSdkSettings,
    azure: azure_endpoints::Settings,
    pub(crate) redactor: Arc<Redactor>,
    client: ProxiedClient,
}

impl Session {
    pub(crate) fn build(text: &str) -> Result<Self, CallError> {
        let module: ModuleSettings = convert::read("assistant session settings", text)?;
        let environment = process_environment();
        let mut settings = resource_sdk_settings::Raw::default()
            .convert(&environment)
            .map_err(|source| CallError::Settings { source })?;
        module.apply(&mut settings);
        let azure = azure_endpoints::Raw::default()
            .convert(&environment)
            .map_err(|source| CallError::Settings { source })?;
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| CallError::Redactor { source })?;
        Ok(Self {
            environment,
            settings: ResourceSdkSettings::from(&settings),
            azure,
            redactor: Arc::new(redactor),
            client,
        })
    }

    pub(crate) async fn message(
        &self,
        call: &MessageCall,
        operation: Operation,
        asynchronous: bool,
    ) -> Result<Reply, MessageFailure> {
        let request = MessageRequest {
            provider: &call.custom_llm_provider,
            thread_id: &call.thread_id,
            data: call.data.as_ref(),
            params: &call.params,
            drop_params: call.drop_params,
        };
        message_call(&request, &self.message_context(), operation, asynchronous).await
    }

    pub(crate) fn message_context(&self) -> MessageContext<'_, ProxiedClient> {
        MessageContext {
            settings: &self.settings,
            azure: &self.azure,
            environment: &self.environment,
            sources: &NoSources,
            transport: Arc::new(self.client.clone()),
            redactor: self.redactor.as_ref(),
        }
    }

    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn run_sdk(&self) -> AssistantRunSdk<'_, ProviderRuns<ProxiedClient>> {
        AssistantRunSdk {
            operations: ProviderRuns {
                transport: Arc::new(self.client.clone()),
            },
            settings: &self.settings,
            azure: &self.azure,
            environment: &self.environment,
            sources: &NoSources,
            redactor: self.redactor.as_ref(),
        }
    }
}
