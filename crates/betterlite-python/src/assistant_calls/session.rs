use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_config::loader::ConfigError;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::request::AssistantReadDeleteInput;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::{
    AssistantSdkError, NoSources, SdkDispatch,
};
use betterlite_core::api::assistant_read_delete::{AssistantReadDelete, AssistantStep};
use betterlite_core::api::assistant_records::AssistantCreateInput;
use betterlite_core::api::assistant_records::error::AssistantCreateFailure;
use betterlite_core::api::assistant_records::service::AssistantRecords;
use betterlite_core::api::assistant_threads::{
    AssistantThreads, CreateThreadCall, GetThreadCall, ThreadFacade as _,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::records::AssistantObject;
use betterlite_openai::threads::records::ThreadRecord;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::convert::{self, ModuleSettings, ThreadRequest};
use super::error::CallError;
use super::error::family::ThreadOutcome;

mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the request redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error("the assistant settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
}

// The two thread functions differ only in the call they build around the same facade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadKind {
    Create,
    Get,
}

// Everything one Python session owns: the environment snapshot, the HTTP client and the values get_llm_provider reads.
pub(crate) struct Session {
    environment: BTreeMap<String, String>,
    settings: resource_sdk_settings::Settings,
    resources: ResourceSdkSettings,
    azure: azure_endpoints::Settings,
    pub(crate) redactor: Redactor,
    catalog: Arc<ModelCatalog>,
    known_models: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    policies: ProviderPolicies,
    globals: GlobalCredentials,
    client: ProxiedClient,
}

impl Session {
    pub(crate) fn build(text: &str) -> Result<Self, CallError> {
        let module = convert::read("assistant session settings", text)?;
        let environment = process_environment();
        let mut settings = resource_sdk_settings::Raw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        apply_module_settings(&mut settings, &module);
        let azure = azure_endpoints::Raw::default()
            .convert(&environment)
            .map_err(|source| SessionError::Settings { source })?;
        let globals = GlobalCredentials {
            api_key: settings.api_key.clone(),
            api_base: settings.api_base.clone(),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        };
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redactor { source })?;
        let catalog = BundledCatalog::default().get();
        let resources = ResourceSdkSettings::from(&settings);
        Ok(Self {
            environment,
            settings,
            resources,
            azure,
            redactor,
            catalog,
            known_models: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals,
            client,
        })
    }

    pub(crate) async fn read_delete(
        &self,
        input: &AssistantReadDeleteInput,
    ) -> Result<AssistantStep, AssistantReadDeleteError<AssistantSdkError>> {
        let dispatch = SdkDispatch {
            transport: Arc::new(self.client.clone()),
            sources: NoSources,
            environment: self.environment.clone(),
            azure: self.azure.clone(),
        };
        let facade = AssistantReadDelete {
            dispatch,
            limits: ApiLimits::default(),
            settings: self.settings.clone(),
        };
        facade.run(input).await.map(|result| result.step)
    }

    pub(crate) async fn create(
        &self,
        input: &AssistantCreateInput,
        asynchronous: bool,
    ) -> Result<AssistantObject, Box<AssistantCreateFailure>> {
        let context = self.resolution();
        let records = AssistantRecords {
            settings: self.settings.clone(),
            azure: self.azure.clone(),
            environment: &self.environment,
            sources: &NoSources,
            resolution: &context,
            redactor: &self.redactor,
            transport: Arc::new(self.client.clone()),
        };
        let outcome = if asynchronous {
            records.acreate_assistants(input).await
        } else {
            records.create_assistants(input).await
        };
        outcome.map(|created| created.assistant)
    }

    pub(crate) async fn thread(
        &self,
        kind: ThreadKind,
        request: &ThreadRequest,
        asynchronous: bool,
    ) -> Result<Option<ThreadRecord>, ThreadOutcome> {
        let facade = self.thread_facade();
        let mut kwargs = request.kwargs.clone();
        match kind {
            ThreadKind::Create => {
                let call = CreateThreadCall {
                    custom_llm_provider: request.custom_llm_provider.clone(),
                    messages: request.messages.clone(),
                    metadata: request.metadata.clone(),
                    tool_resources: request.tool_resources.clone(),
                    client: None,
                    kwargs: &mut kwargs,
                };
                if asynchronous {
                    let answer = facade.acreate_thread(call).await;
                    answer.map_err(ThreadOutcome::Async)
                } else {
                    let answer = facade.create_thread(call).await;
                    answer.map_err(ThreadOutcome::Sync)
                }
            }
            ThreadKind::Get => {
                let call = GetThreadCall {
                    custom_llm_provider: request.custom_llm_provider.clone(),
                    thread_id: request.thread_id.clone().unwrap_or_default(),
                    client: None,
                    kwargs: &mut kwargs,
                };
                if asynchronous {
                    let answer = facade.aget_thread(call).await;
                    answer.map_err(ThreadOutcome::Async)
                } else {
                    let answer = facade.get_thread(call).await;
                    answer.map_err(ThreadOutcome::Sync)
                }
            }
        }
    }

    fn thread_facade(&self) -> AssistantThreads<'_, ProxiedClient> {
        AssistantThreads {
            transport: Arc::new(self.client.clone()),
            resources: &self.resources,
            azure: &self.azure,
            environment: &self.environment,
            sources: &NoSources,
            redactor: &self.redactor,
            refresh_enabled: false,
            session: SessionKind::Dedicated,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: self.catalog.as_ref(),
            known_model_names: &self.known_models,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: self.policies.clone(),
            globals: self.globals.clone(),
        }
    }
}

// litellm.api_key and the other module globals win over the environment, and an unset one leaves it in place.
fn apply_module_settings(settings: &mut resource_sdk_settings::Settings, module: &ModuleSettings) {
    settings.api_key.clone_from(&module.api_key);
    settings.api_base.clone_from(&module.api_base);
    settings.api_version.clone_from(&module.api_version);
    settings.organization.clone_from(&module.organization);
    settings.openai_key.clone_from(&module.openai_key);
    settings.azure_key.clone_from(&module.azure_key);
}

// the outbound client honours the proxy variables of the environment the session was built from
fn outbound_client(environment: &BTreeMap<String, String>) -> Result<ProxiedClient, SessionError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(|source| SessionError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| SessionError::Client { source })
}
