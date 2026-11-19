#![cfg(test)]

mod model_prefix_and_bridge_parameters;
mod prompt_hook_mutation_order;
mod prompt_swap_credential_boundary;
mod provider_capability_selection;
mod supplied_file_mapping;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::ParameterMap;
use super::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptManager,
    PromptManagerError,
};
use super::provider_config::{ConfigLookup, NativeConfig};
use super::resolution::ModelResolver;
use crate::api::responses_parameters::prompt_merge::ReturnedMessage;

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the test JSON parses")
}

fn parameters(pairs: &[(&str, JsonValue)]) -> ParameterMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn capable_native() -> NativeConfig {
    NativeConfig {
        encrypted_agent_messages: true,
    }
}

struct ScriptedConfigs {
    native: BTreeMap<String, NativeConfig>,
}

fn configs(entries: &[(&str, NativeConfig)]) -> ScriptedConfigs {
    ScriptedConfigs {
        native: entries
            .iter()
            .map(|(provider, config)| ((*provider).to_owned(), *config))
            .collect(),
    }
}

fn capabilities_configs() -> ScriptedConfigs {
    configs(&[("openai", capable_native())])
}

impl ConfigLookup for ScriptedConfigs {
    fn responses_api_config(
        &self,
        _model: &str,
        provider: &str,
        _api_base: Option<&str>,
    ) -> Option<NativeConfig> {
        self.native.get(provider).copied()
    }
}

#[derive(Clone)]
struct ScriptedResolution {
    model: String,
    provider: String,
    api_key: Option<String>,
    api_base: Option<String>,
}

fn resolution(model: &str, provider: &str) -> ScriptedResolution {
    ScriptedResolution {
        model: model.to_owned(),
        provider: provider.to_owned(),
        api_key: None,
        api_base: None,
    }
}

fn dynamic_resolution(
    model: &str,
    provider: &str,
    api_key: Option<&str>,
    api_base: Option<&str>,
) -> ScriptedResolution {
    ScriptedResolution {
        model: model.to_owned(),
        provider: provider.to_owned(),
        api_key: api_key.map(str::to_owned),
        api_base: api_base.map(str::to_owned),
    }
}

struct ResolutionCall {
    model: String,
    custom_llm_provider: Option<String>,
    api_base: Option<String>,
    api_key: Option<String>,
}

struct ScriptedResolver {
    answers: BTreeMap<String, ScriptedResolution>,
    calls: Mutex<Vec<ResolutionCall>>,
}

fn resolver(answers: &[(&str, ScriptedResolution)]) -> ScriptedResolver {
    ScriptedResolver {
        answers: answers
            .iter()
            .map(|(model, answer)| ((*model).to_owned(), answer.clone()))
            .collect(),
        calls: Mutex::new(Vec::new()),
    }
}

impl ScriptedResolver {
    fn calls(&self) -> Vec<ResolutionCall> {
        self.calls
            .lock()
            .expect("the resolver call log lock")
            .drain(..)
            .collect()
    }

    fn models(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.model).collect()
    }
}

impl ModelResolver for ScriptedResolver {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        self.calls
            .lock()
            .expect("the resolver call log lock")
            .push(ResolutionCall {
                model: request.model.to_owned(),
                custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
                api_base: request.api_base.map(str::to_owned),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            });
        let answer =
            self.answers
                .get(request.model)
                .ok_or_else(|| ResolutionError::MissingProvider {
                    model: request.model.to_owned(),
                })?;
        Ok(ResolvedModel {
            model: answer.model.clone(),
            provider: ProviderIdentity::Custom(answer.provider.clone()),
            api_base: answer.api_base.clone(),
            api_key: answer.api_key.clone().map(SecretString::from),
        })
    }
}

enum HookBehavior {
    Skip,
    Answer(MergeAnswer),
    Fail,
}

struct HookRecord {
    model: String,
    messages: Vec<JsonValue>,
    prompt_id: Option<String>,
    prompt_label: Option<String>,
    prompt_version: Option<JsonValue>,
    prompt_variables: Option<JsonValue>,
    signal: Option<JsonValue>,
}

struct HookRequestRecord {
    prompt_id: Option<String>,
    non_default_params: usize,
}

struct ScriptedManager {
    behavior: HookBehavior,
    calls: Mutex<Vec<HookRecord>>,
    requests: Mutex<Vec<HookRequestRecord>>,
}

fn manager(behavior: HookBehavior) -> ScriptedManager {
    ScriptedManager {
        behavior,
        calls: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
    }
}

fn hook_answer(
    model: &str,
    messages: &[JsonValue],
    optional_params: &[(&str, JsonValue)],
) -> MergeAnswer {
    MergeAnswer {
        model: model.to_owned(),
        messages: messages
            .iter()
            .map(|message| ReturnedMessage {
                source: None,
                message: message.clone(),
            })
            .collect(),
        optional_params: parameters(optional_params),
    }
}

impl ScriptedManager {
    fn calls(&self) -> Vec<HookRecord> {
        self.calls
            .lock()
            .expect("the hook call log lock")
            .drain(..)
            .collect()
    }

    fn requests(&self) -> Vec<HookRequestRecord> {
        self.requests
            .lock()
            .expect("the hook request log lock")
            .drain(..)
            .collect()
    }
}

impl PromptManager for ScriptedManager {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool {
        self.requests
            .lock()
            .expect("the hook request log lock")
            .push(HookRequestRecord {
                prompt_id: request.prompt_id.map(str::to_owned),
                non_default_params: request.non_default_params.len(),
            });
        !matches!(self.behavior, HookBehavior::Skip)
    }

    fn get_chat_completion_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        self.calls
            .lock()
            .expect("the hook call log lock")
            .push(HookRecord {
                model: request.model.to_owned(),
                messages: request
                    .messages
                    .iter()
                    .map(|message| message.message.clone())
                    .collect(),
                prompt_id: request.prompt_id.map(str::to_owned),
                prompt_label: request.prompt_label.map(str::to_owned),
                prompt_version: request.prompt_version.cloned(),
                prompt_variables: request.prompt_variables.cloned(),
                signal: request
                    .request_kwargs
                    .get(CARRY_UNMATCHED_MESSAGE_POINTS)
                    .cloned(),
            });
        match &self.behavior {
            HookBehavior::Answer(answer) => Ok(answer.clone()),
            HookBehavior::Skip => Err(PromptManagerError {
                message: String::from("the hook must not run"),
            }),
            HookBehavior::Fail => Err(PromptManagerError {
                message: String::from("boom"),
            }),
        }
    }
}
