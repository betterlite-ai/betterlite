use super::calls::ProviderCalls;
use super::facts::{CatalogFacts, SessionMetadata};
use super::ports::{NoLogging, NoPricing, NoPrompt, SessionTimeouts};
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::chat::ChatCall;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_wrapper::ports::{IdSource, WrapperClock};
use betterlite_sdk::completion_call::bridge_facts::ModelBridgeFacts;
use betterlite_sdk::completion_call::ports::{NoFoundryRules, NoMcpLoop, Sleeper};
use betterlite_sdk::completion_call::target::CompletionTarget;
use betterlite_sdk::completion_request::args::CompletionSettings;
use betterlite_sdk::completion_request::ports::CompletionPorts;
use std::sync::Arc;
// What a finished target needs from the session: the deployments it may dispatch to and the ports around it.
pub(crate) struct TargetSpec {
    pub(crate) settings: CompletionSettings,
    pub(crate) deployments: Vec<ChatDeployment>,
    pub(crate) redactor: Redactor,
    pub(crate) client: ProxiedClient,
    pub(crate) capabilities: CatalogCapabilities,
    pub(crate) ids: Arc<dyn IdSource>,
    pub(crate) clock: Arc<dyn WrapperClock>,
}
// The session-owned ports a target borrows; the caller keeps the scope alive for as long as the target lives.
pub(crate) struct CallScope<'core> {
    pub(crate) metadata: SessionMetadata<'core>,
    pub(crate) bridge: ModelBridgeFacts<'core, NoFoundryRules>,
    pub(crate) sleeper: &'core dyn Sleeper,
}
// One place builds every target, so the call path and the probes run through the same port set.
pub(crate) fn build<'scope>(
    scope: &'scope CallScope<'scope>,
    spec: TargetSpec,
) -> CompletionTarget<'scope> {
    let calls: Arc<dyn ChatCall> = Arc::new(ProviderCalls {
        client: spec.client,
        catalog: spec.capabilities,
    });
    CompletionTarget {
        settings: spec.settings,
        ports: CompletionPorts {
            model_metadata: &scope.metadata,
            prompt: &NoPrompt,
            bridge: &scope.bridge,
            pricing: &NoPricing,
            logging: &NoLogging,
            timeout: &SessionTimeouts,
        },
        dispatch: Arc::new(Dispatch::new(
            spec.deployments,
            Arc::new(NoRules),
            Arc::new(TracingCallLog),
        )),
        calls,
        mock: MockSettings::default(),
        ids: spec.ids,
        clock: spec.clock,
        redactor: spec.redactor,
        mcp: &NoMcpLoop,
        sleeper: scope.sleeper,
        stream_bytes: TransportLimits::DEFAULT.body_bytes,
    }
}
// No foundry deployment reaches a Python session from here; the OpenAI credentials are the session's own.
pub(crate) fn bridge(facts: &CatalogFacts) -> ModelBridgeFacts<'_, NoFoundryRules> {
    ModelBridgeFacts {
        facts,
        foundry: &NoFoundryRules,
        openai: OpenAiCredentialsSettings::default(),
        litellm_api_base: None,
    }
}
