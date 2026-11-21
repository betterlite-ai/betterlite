use super::convert::{
    CallWire, OutcomeParts, ProbeCall, SessionWire, TokenCall, outcome_parts, parse,
};
use super::deployments;
use super::error::SessionError;
use super::facts::{CatalogFacts, SessionMetadata};
use super::ports::{IdentityStream, LocalCounter, SystemClock, SystemIds, TokioSleeper};
use super::setup::{
    DEFAULT_MEDIUM_THINKING_BUDGET, global_credentials, openai_defaults, redactor, runtime_config,
};
use super::targets::{self, TargetPorts, TargetSpec};
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_core::environment_validation::probe::{check_valid_key, valid_model};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::text_completion_execution::catalog::CompletionCatalog;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_wrapper::error::{CallError, plain_error};
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use betterlite_sdk::call_wrapper::target::CallTarget as _;
use betterlite_sdk::completion_call::error::into_call_error;
use betterlite_sdk::completion_call::probe::CompletionProbe;
use betterlite_sdk::completion_call::target_async::AsyncCompletionTarget;
use betterlite_sdk::completion_call::tokens::{
    TokenCountRequest, TokenCountResponse, count_tokens,
};
use betterlite_sdk::completion_request::args::CompletionSettings;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::runtime::runtime_counter;
use secrecy::ExposeSecret as _;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
const PROBE_TIMEOUT: Duration = Duration::from_secs(600);
// One probe, two questions: litellm.utils.valid_model and utils.check_valid_key differ only in what they ask.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ProbeCheck {
    Model,
    Key,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeAnswer {
    Valid,
    Invalid,
}
// Everything one Python session owns: the environment snapshot, the HTTP client and the catalog it resolves against.
pub(crate) struct SessionCore {
    environment: BTreeMap<String, String>,
    catalog: Arc<ModelCatalog>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    pub(crate) defaults: OpenAiDefaults,
    client: ProxiedClient,
    counter: Counter,
    capabilities: CatalogCapabilities,
    model_sets: ProviderModelSets,
    ids: Arc<dyn IdSource>,
    clock: Arc<dyn WrapperClock>,
    sleeper: TokioSleeper,
    medium_thinking_budget: u32,
}
impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let policy =
            ProxyPolicy::resolve(&ProxyTrust::default(), &environment).map_err(|error| {
                SessionError::Proxy {
                    message: error.to_string(),
                }
            })?;
        let client =
            ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| SessionError::Client {
                message: error.to_string(),
            })?;
        let catalog = BundledCatalog::default().get();
        let completion = CompletionCatalog::new(&catalog);
        let mut model_sets = ProviderModelSets::default();
        model_sets.add_known_models(&catalog);
        let counter = runtime_counter(&runtime_config(), &Arc::new(client.clone()), &catalog)
            .map_err(|error| SessionError::Tokenizer {
                message: error.to_string(),
            })?;
        Ok(Self {
            defaults: openai_defaults(&environment),
            environment,
            catalog,
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            client,
            counter,
            capabilities: CatalogCapabilities {
                chat_models: completion.chat_models,
                text_models: completion.text_models,
            },
            model_sets,
            ids: Arc::new(SystemIds),
            clock: Arc::new(SystemClock::new()),
            sleeper: TokioSleeper,
            medium_thinking_budget: wire
                .medium_thinking_budget
                .unwrap_or(DEFAULT_MEDIUM_THINKING_BUDGET),
        })
    }
    pub(crate) fn resolve(
        &self,
        lookup: &ProviderLookup<'_>,
        globals: &GlobalCredentials,
    ) -> Result<ResolvedModel, ResolutionError> {
        resolve(
            &ResolutionRequest {
                model: lookup.model,
                custom_llm_provider: lookup.custom_llm_provider,
                api_base: lookup.api_base,
                api_key: lookup.api_key,
            },
            &self.context(globals),
        )
    }
    // the resolution context carries one snapshot of every outside value a get_llm_provider call reads
    pub(crate) fn context(&self, globals: &GlobalCredentials) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: globals.clone(),
        }
    }
    pub(crate) async fn call(
        &self,
        wire: CallWire,
        acompletion: bool,
    ) -> Result<OutcomeParts, CallError> {
        let settings = wire.settings.into_settings(self.medium_thinking_budget);
        let globals = global_credentials(&settings);
        let spec = TargetSpec {
            deployments: deployments::all(self, &wire.arguments, &settings, &globals),
            redactor: redactor()?,
            client: self.client.clone(),
            capabilities: self.capabilities.clone(),
            ids: Arc::clone(&self.ids),
            clock: Arc::clone(&self.clock),
            settings,
        };
        let metadata = SessionMetadata {
            core: self,
            globals,
        };
        let facts = CatalogFacts {
            catalog: &self.catalog,
        };
        let bridge = targets::bridge(&facts);
        let target = targets::build(
            TargetPorts {
                metadata: &metadata,
                facts: &facts,
                bridge: &bridge,
                sleeper: &self.sleeper,
            },
            spec,
        );
        let outcome = if acompletion {
            let callbacks: Vec<Arc<dyn AgenticCallback>> = Vec::new();
            let target = AsyncCompletionTarget {
                inner: target,
                callbacks: &callbacks,
                fake_stream: &IdentityStream,
            };
            target.call(wire.arguments, None).await
        } else {
            target.call(wire.arguments, None).await
        }?;
        outcome_parts(outcome).map_err(|error| CallError::from(plain_error(&error.to_string())))
    }
    pub(crate) async fn count(&self, call: TokenCall) -> Result<TokenCountResponse, CallError> {
        let metadata = SessionMetadata {
            core: self,
            globals: GlobalCredentials::default(),
        };
        let request = TokenCountRequest {
            model: &call.model,
            messages: call.messages.as_deref(),
            tools: call.tools.as_deref(),
            system: call.system.as_deref(),
            api_key: call.api_key.as_ref(),
            api_base: call.api_base.as_deref(),
        };
        let local = LocalCounter {
            counter: &self.counter,
        };
        let redactor = redactor()?;
        count_tokens(&request, &metadata, None, &local)
            .await
            .map_err(|error| into_call_error(&error, &call.model, "", &redactor))
    }
    pub(crate) async fn probe(
        &self,
        call: &ProbeCall,
        check: ProbeCheck,
    ) -> Result<ProbeAnswer, CallError> {
        let settings = CompletionSettings::default();
        let globals = global_credentials(&settings);
        let mut arguments = RequestMap::new();
        arguments.insert(String::from("model"), Json::Text(call.model.clone()));
        if let Some(key) = &call.api_key {
            arguments.insert(
                String::from("api_key"),
                Json::Text(key.expose_secret().to_owned()),
            );
        }
        let deployments = deployments::all(self, &arguments, &settings, &globals);
        let Some(found) = deployments.first() else {
            return Ok(ProbeAnswer::Invalid);
        };
        let api_base = Some(found.api_base.clone());
        let api_key = found.api_key.clone();
        let spec = TargetSpec {
            settings,
            deployments,
            redactor: redactor()?,
            client: self.client.clone(),
            capabilities: self.capabilities.clone(),
            ids: Arc::clone(&self.ids),
            clock: Arc::clone(&self.clock),
        };
        let metadata = SessionMetadata {
            core: self,
            globals,
        };
        let facts = CatalogFacts {
            catalog: &self.catalog,
        };
        let bridge = targets::bridge(&facts);
        let target = targets::build(
            TargetPorts {
                metadata: &metadata,
                facts: &facts,
                bridge: &bridge,
                sleeper: &self.sleeper,
            },
            spec,
        );
        let probe = CompletionProbe {
            client: &self.client,
            completion: &target,
            api_base,
            api_key: api_key.clone(),
            timeout: PROBE_TIMEOUT,
        };
        let answer = match check {
            ProbeCheck::Model => valid_model(&probe, &self.model_sets, &call.model)
                .await
                .is_ok(),
            ProbeCheck::Key => match &api_key {
                Some(key) => check_valid_key(&probe, &call.model, key).await,
                None => false,
            },
        };
        Ok(if answer {
            ProbeAnswer::Valid
        } else {
            ProbeAnswer::Invalid
        })
    }
}
