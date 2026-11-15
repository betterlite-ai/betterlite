#![cfg(test)]

use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use secrecy::SecretString;

use crate::registry::mutation::prompt_registry_mutation::callbacks::PromptCallbackManager;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallbackCall {
    Added(PromptManagementClient),
    Removed(PromptManagementClient),
}

#[derive(Debug, Default)]
pub(crate) struct RecordingCallbacks {
    pub(crate) calls: Vec<CallbackCall>,
}

impl RecordingCallbacks {
    pub(crate) fn added(&self) -> Vec<PromptManagementClient> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                CallbackCall::Added(callback) => Some(callback.clone()),
                CallbackCall::Removed(_) => None,
            })
            .collect()
    }

    pub(crate) fn removed(&self) -> Vec<PromptManagementClient> {
        self.calls
            .iter()
            .filter_map(|call| match call {
                CallbackCall::Removed(callback) => Some(callback.clone()),
                CallbackCall::Added(_) => None,
            })
            .collect()
    }
}

impl PromptCallbackManager for RecordingCallbacks {
    fn add_litellm_callback(&mut self, callback: &PromptManagementClient) {
        self.calls.push(CallbackCall::Added(callback.clone()));
    }

    fn remove_callback_from_all_lists(&mut self, callback: &PromptManagementClient) {
        self.calls.push(CallbackCall::Removed(callback.clone()));
    }
}

pub(crate) fn prompt_spec(prompt_id: &str, environment: &str, integration: &str) -> PromptSpec {
    let mut spec = crate::tests::prompt_spec(prompt_id, None, environment);
    spec.litellm_params.prompt_integration = integration.to_owned();
    spec
}

pub(crate) fn params(integration: &str) -> PromptLiteLLMParams {
    let mut params = crate::tests::prompt_params();
    params.prompt_integration = integration.to_owned();
    params
}

pub(crate) fn client(prompt_id: Option<&str>) -> PromptManagementClient {
    PromptManagementClient {
        prompt_id: prompt_id.map(str::to_owned),
        prompt_template: Vec::new(),
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    }
}

pub(crate) fn api_key(secret: &str) -> Option<SecretString> {
    Some(SecretString::from(secret.to_owned()))
}

pub(crate) fn registry_with(
    prompt: &PromptSpec,
    callbacks: &mut RecordingCallbacks,
) -> InMemoryPromptRegistry {
    let mut registry = InMemoryPromptRegistry::default();
    registry
        .initialize_prompt(prompt, None, callbacks)
        .expect("the fixture prompt registers");
    callbacks.calls.clear();
    registry
}
