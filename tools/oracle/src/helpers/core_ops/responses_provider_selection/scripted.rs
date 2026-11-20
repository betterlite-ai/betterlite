use betterlite_core::api::responses_parameters::prompt_merge::ReturnedMessage;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_core::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptManager,
    PromptManagerError,
};
use betterlite_core::api::responses_provider_selection::provider_config::{
    ConfigLookup, NativeConfig,
};
use betterlite_core::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Mutex, PoisonError};
#[derive(Debug, Clone, Deserialize)]
pub(super) struct Hook {
    mode: HookMode,
    model: String,
    messages: Vec<JsonValue>,
    optional_params: ParameterMap,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HookMode {
    Answer,
    Skip,
    Throw,
}
pub(super) struct Registry;
impl ConfigLookup for Registry {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        matches!(provider, "openai" | "azure" | "xai").then_some(NativeConfig {
            encrypted_agent_messages: matches!(provider, "openai" | "azure"),
        })
    }
}
pub(super) struct ModelNames;
impl ModelResolver for ModelNames {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let Some(provider @ ("openai" | "anthropic")) =
            request.model.split_once('/').map(|(prefix, _rest)| prefix)
        else {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        };
        Ok(ResolvedModel {
            model: request.model.to_owned(),
            provider: ProviderIdentity::Custom(provider.to_owned()),
            api_base: None,
            api_key: None,
        })
    }
}
pub(super) struct ScriptedManager {
    hook: Option<Hook>,
    calls: Mutex<Vec<Value>>,
    requests: Mutex<Vec<Value>>,
}
impl ScriptedManager {
    pub(super) fn new(hook: Option<&Hook>) -> Self {
        Self {
            hook: hook.cloned(),
            calls: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        }
    }
    pub(super) fn records(&self) -> (Vec<Value>, Vec<Value>) {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        (calls, requests)
    }
}
impl PromptManager for ScriptedManager {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "prompt_id": request.prompt_id,
                "param_count": request.non_default_params.len(),
            }));
        matches!(
            self.hook.as_ref(),
            Some(Hook {
                mode: HookMode::Answer | HookMode::Throw,
                ..
            })
        )
    }
    fn get_chat_completion_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        let messages: Vec<&JsonValue> = request
            .messages
            .iter()
            .map(|message| &message.message)
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "model": request.model,
                "messages": messages,
                "prompt_id": request.prompt_id,
                "prompt_label": request.prompt_label,
                "prompt_version": request.prompt_version,
                "prompt_variables": request.prompt_variables,
                "signal": request.request_kwargs.get(CARRY_UNMATCHED_MESSAGE_POINTS),
            }));
        let Some(hook) = self.hook.as_ref() else {
            return Err(PromptManagerError {
                message: String::from("the hook must not run"),
            });
        };
        match hook.mode {
            HookMode::Answer => Ok(MergeAnswer {
                model: hook.model.clone(),
                messages: returned_messages(&hook.messages),
                optional_params: hook.optional_params.clone(),
            }),
            HookMode::Skip => Err(PromptManagerError {
                message: String::from("the hook must not run"),
            }),
            HookMode::Throw => Err(PromptManagerError {
                message: String::from("boom"),
            }),
        }
    }
}
fn returned_messages(messages: &[JsonValue]) -> Vec<ReturnedMessage> {
    messages
        .iter()
        .map(|message| ReturnedMessage {
            source: None,
            message: message.clone(),
        })
        .collect()
}
