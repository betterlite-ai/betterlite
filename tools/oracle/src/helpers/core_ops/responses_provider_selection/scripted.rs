use std::sync::{Mutex, PoisonError};

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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
        match provider {
            "openai" | "azure" => Some(NativeConfig {
                encrypted_agent_messages: true,
            }),
            "xai" => Some(NativeConfig {
                encrypted_agent_messages: false,
            }),
            _ => None,
        }
    }
}

pub(super) struct ModelNames;

impl ModelResolver for ModelNames {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let Some(provider) = request
            .model
            .split_once('/')
            .map(|(prefix, _rest)| prefix)
            .filter(|prefix| *prefix == "openai" || *prefix == "anthropic")
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

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        match self.hook.as_ref() {
            Some(Hook {
                mode: HookMode::Answer,
                model,
                messages,
                optional_params,
            }) => Ok(MergeAnswer {
                model: model.clone(),
                messages: messages
                    .iter()
                    .map(|message| ReturnedMessage {
                        source: None,
                        message: message.clone(),
                    })
                    .collect(),
                optional_params: optional_params.clone(),
            }),
            None
            | Some(Hook {
                mode: HookMode::Skip,
                ..
            }) => Err(PromptManagerError {
                message: String::from("the hook must not run"),
            }),
            Some(Hook {
                mode: HookMode::Throw,
                ..
            }) => Err(PromptManagerError {
                message: String::from("boom"),
            }),
        }
    }
}
