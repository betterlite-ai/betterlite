use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_config::api::responses_provider_selection::compatible_provider_settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::responses_parameters::prompt_merge::ReturnedMessage;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_core::api::responses_provider_selection::bridging::bridge_kwargs;
use betterlite_core::api::responses_provider_selection::error::CredentialBoundaryError;
use betterlite_core::api::responses_provider_selection::file_ids::apply_managed_file_id_mapping;
use betterlite_core::api::responses_provider_selection::prefixes::{
    normalize_openai_chat_completions_responses_model, pop_use_chat_completions_api,
    strip_responses_routing_prefix,
};
use betterlite_core::api::responses_provider_selection::prompting::bridge::{
    ProvisionalBridge, api_base_kwarg, will_bridge_to_chat_completions,
};
use betterlite_core::api::responses_provider_selection::prompting::swap::{
    PromptSwapError, SwapRequest, deployment_model_info_after_prompt_swap,
    resolve_prompt_swapped_provider,
};
use betterlite_core::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptError,
    PromptManager, PromptManagerError, PromptRequest, apply_prompt_management_to_responses_call,
};
use betterlite_core::api::responses_provider_selection::provider_config::{
    ASYNC_PROMPT_MERGED_PARAMS, ConfigLookup, ConfigRequest, NativeConfig, ResponsesApiConfig,
    bridges_to_chat_completions, deployment_passes_through_responses,
    encrypted_task_support_failure, resolve_responses_api_provider_config,
};
use betterlite_core::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_openai::response_compatible::OpenAiLikeResponsesConfig;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::responses::ResponsesInput;
use betterlite_prompts::file_ids::tools::ResponsesTool;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Call {
    Selection(Selection),
    Prefixes(Prefixes),
    Compatible(Compatible),
    CredentialBoundary(CredentialBoundary),
    PromptCall(PromptCall),
    FileMapping(FileMapping),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    provider: String,
    model: String,
    use_chat_completions_api: bool,
    model_info: JsonValue,
    kwargs: ParameterMap,
    allowed_openai_params: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prefixes {
    model: String,
    flag_present: bool,
    flag: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compatible {
    environment: BTreeMap<String, String>,
    api_key: Option<String>,
    api_base: Option<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialBoundary {
    original_model: String,
    swapped_model: String,
    provider: Option<String>,
    prompt_id: Option<String>,
    kwargs: ParameterMap,
    model_info: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptCall {
    input: JsonValue,
    model: String,
    provider: Option<String>,
    use_chat_completions_api: bool,
    logging: bool,
    hook: Option<Hook>,
    async_outcome: Option<JsonValue>,
    kwargs: ParameterMap,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hook {
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMapping {
    input: ResponsesInput,
    tools: Option<Vec<ResponsesTool>>,
    model_info: Option<JsonValue>,
    mapping: Option<FileIdMapping>,
}

struct Registry;

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

struct ModelNames;

impl ModelResolver for ModelNames {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let Some((prefix, _rest)) = request.model.split_once('/') else {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        };
        match prefix {
            "openai" | "anthropic" => Ok(ResolvedModel {
                model: request.model.to_owned(),
                provider: ProviderIdentity::Custom(prefix.to_owned()),
                api_base: None,
                api_key: None,
            }),
            _ => Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            }),
        }
    }
}

struct ScriptedManager {
    hook: Option<Hook>,
    calls: Mutex<Vec<Value>>,
    requests: Mutex<Vec<Value>>,
}

impl ScriptedManager {
    fn new(hook: Option<&Hook>) -> Self {
        Self {
            hook: hook.cloned(),
            calls: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PromptManager for ScriptedManager {
    fn should_run_prompt_management_hooks(&self, request: &HookRequest<'_>) -> bool {
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

fn prompt_error(error: PromptError) -> Box<HelperError> {
    match error {
        PromptError::Swap(PromptSwapError::CredentialBoundary(boundary)) => {
            boundary_error(&boundary)
        }
        PromptError::Swap(other) => value_error(other.to_string()),
        PromptError::Merge(error) => value_error(error.to_string()),
        PromptError::Manager(error) => value_error(error.to_string()),
        PromptError::AsyncOutcome(error) => value_error(error.to_string()),
    }
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}

fn boundary_error(boundary: &CredentialBoundaryError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("BadRequestError"),
        status: Some(400),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: boundary.message.clone(),
    })
}

fn selection(case: &Selection) -> HelperResult {
    let config = resolve_responses_api_provider_config(
        &Registry,
        &ConfigRequest {
            model: &case.model,
            provider: &case.provider,
            model_info: &case.model_info,
            api_base: api_base_kwarg(&case.kwargs),
        },
    );
    let config_kind = match &config {
        None => Value::Null,
        Some(ResponsesApiConfig::Native(_native)) => Value::String(String::from("native")),
        Some(ResponsesApiConfig::OpenAiLike) => Value::String(String::from("openai_like")),
    };
    let encrypted = encrypted_task_support_failure(config.as_ref(), case.use_chat_completions_api)
        .map_or(Value::Null, |failure| {
            Value::String(String::from(failure.wire_name()))
        });
    let api_base = api_base_kwarg(&case.kwargs);
    let will_bridge = will_bridge_to_chat_completions(
        &Registry,
        &ProvisionalBridge {
            model: &case.model,
            custom_llm_provider: Some(&case.provider),
            use_chat_completions_api: case.use_chat_completions_api,
            model_info: &case.model_info,
            api_base,
        },
    );
    let kept = bridge_kwargs(&case.kwargs, config.as_ref(), &case.allowed_openai_params);
    Ok(json!({
        "config": config_kind,
        "bridges": bridges_to_chat_completions(config.as_ref(), case.use_chat_completions_api),
        "encrypted": encrypted,
        "passes": deployment_passes_through_responses(&case.model_info),
        "api_base": api_base,
        "bridge_kwargs": kept,
        "will_bridge": will_bridge,
    }))
}

fn prefixes(case: &Prefixes) -> HelperResult {
    let mut kwargs = ParameterMap::new();
    if case.flag_present {
        kwargs.insert(
            String::from("use_chat_completions_api"),
            case.flag.clone().unwrap_or(JsonValue::Null),
        );
    }
    let popped = pop_use_chat_completions_api(&mut kwargs);
    let (normalized, prefixed) = normalize_openai_chat_completions_responses_model(&case.model);
    Ok(json!({
        "normalized": [normalized, prefixed],
        "stripped": strip_responses_routing_prefix(&case.model),
        "popped": popped,
        "remaining": kwargs,
    }))
}

fn compatible(case: &Compatible) -> HelperResult {
    let loaded = ConfigLoader {
        lookup: case.environment.clone(),
    }
    .load("")
    .or_config_error()?;
    let config = OpenAiLikeResponsesConfig {
        settings: compatible_provider_settings(&loaded).clone(),
    };
    let explicit_key = case
        .api_key
        .as_ref()
        .map(|key| SecretString::from(String::from(key.as_str())));
    let headers = config.validate_environment(case.headers.clone(), explicit_key.as_ref());
    let url = config
        .get_complete_url(case.api_base.as_deref())
        .or_value_error()?;
    let identity = config.identity();
    let name: &'static str = (&identity.custom_llm_provider).into();
    Ok(json!({
        "identity": name,
        "native_file_search": identity.supports_native_file_search,
        "encrypted": identity.encrypted_agent_messages,
        "headers": headers,
        "url": url,
    }))
}

fn credential_boundary(case: &CredentialBoundary) -> HelperResult {
    let provider = resolve_prompt_swapped_provider(
        &SwapRequest {
            original_model: &case.original_model,
            swapped_model: &case.swapped_model,
            custom_llm_provider: case.provider.as_deref(),
            prompt_id: case.prompt_id.as_deref(),
        },
        &case.kwargs,
        &ModelNames,
    )
    .map_err(prompt_error)?;
    let model_info = deployment_model_info_after_prompt_swap(
        case.provider.as_deref(),
        Some(provider.as_str()),
        case.model_info.clone(),
    );
    Ok(json!({"provider": provider, "model_info": model_info}))
}

fn prompt_call(case: &PromptCall) -> HelperResult {
    let mut kwargs = case.kwargs.clone();
    if let Some(outcome) = &case.async_outcome {
        kwargs.insert(String::from(ASYNC_PROMPT_MERGED_PARAMS), outcome.clone());
    }
    let manager = if case.logging {
        Some(ScriptedManager::new(case.hook.as_ref()))
    } else {
        None
    };
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management_to_responses_call(
        PromptRequest {
            input: case.input.clone(),
            model: &case.model,
            custom_llm_provider: case.provider.as_deref(),
            use_chat_completions_api: case.use_chat_completions_api,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        manager
            .as_ref()
            .map(|scripted| scripted as &dyn PromptManager),
        &Registry,
        &ModelNames,
    )
    .map_err(prompt_error)?;
    let (requests, calls) = manager
        .as_ref()
        .map_or((Vec::new(), Vec::new()), |scripted| {
            (scripted.requests(), scripted.calls())
        });
    Ok(json!({
        "input": outcome.input,
        "model": outcome.model,
        "provider": outcome.custom_llm_provider,
        "deployment_model_info": outcome.deployment_model_info,
        "locals": local_vars,
        "kwargs": kwargs,
        "requests": requests,
        "calls": calls,
    }))
}

fn file_mapping(case: &FileMapping) -> HelperResult {
    let mut local_vars = ParameterMap::new();
    let outcome = apply_managed_file_id_mapping(
        &case.input,
        case.tools.as_deref(),
        case.model_info.as_ref(),
        case.mapping.as_ref(),
        &mut local_vars,
    )
    .or_value_error()?;
    Ok(json!({
        "input": outcome.input,
        "tools": outcome.tools,
        "locals": local_vars,
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Call::Selection(case) => selection(case),
            Call::Prefixes(case) => prefixes(case),
            Call::Compatible(case) => compatible(case),
            Call::CredentialBoundary(case) => credential_boundary(case),
            Call::PromptCall(case) => prompt_call(case),
            Call::FileMapping(case) => file_mapping(case),
        }
    }
}
