use std::error::Error;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_models::resolution::ResolvedModel;
use betterlite_models::resolution::resolve;
use betterlite_prompts::arguments::json::{Json, Object};

use super::config::GenerateContentProviderConfig;
use super::config::GenerateContentProviderConfigs;
use super::config::GenerateContentRequest;
use super::config::provider_config_for;
use super::error::GenAiSetupError;
use super::logging::CallLog;
use super::logging::LoggingUpdateError;
use super::logging::log_pre_call;
use super::params::GenerateContentLiteLLMParams;
use super::params::RequestFieldError;
use super::params::optional_string;

pub struct SetupRequest<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    pub config: Option<&'request Object>,
    pub custom_llm_provider: Option<&'request str>,
    pub tools: Option<&'request Json>,
    pub kwargs: &'request Object,
}

// GenerateContentSetupResult: every value the call handlers read back off the setup
pub struct GenerateContentSetupResult<'configs, 'call, C> {
    pub model: String,
    pub custom_llm_provider: String,
    pub request_body: Json,
    pub provider_config: Option<&'configs C>,
    pub config_dict: Object,
    pub native_request_fields: Object,
    pub litellm_params: GenerateContentLiteLLMParams,
    pub logging: &'call mut LoggingState,
    pub litellm_call_id: Option<String>,
}

// the resolved provider and model plus the params the result carries back
struct ResolvedCall {
    model: String,
    provider: String,
    litellm_params: GenerateContentLiteLLMParams,
    call_id: Option<String>,
}

// setup_generate_content_call: resolution, provider dispatch, the native request, and the adapter fallback
pub fn setup_generate_content_call<'call, 'configs, C>(
    request: &SetupRequest<'_>,
    configs: &'configs GenerateContentProviderConfigs<'configs, C>,
    logging: Option<&'call mut LoggingState>,
    resolution: &ResolutionContext<'_>,
) -> Result<GenerateContentSetupResult<'configs, 'call, C>, GenAiSetupError<C::Error>>
where
    C: GenerateContentProviderConfig,
{
    let mut litellm_params = GenerateContentLiteLLMParams::from_kwargs(request.kwargs)?;
    reject_string_mock(request.kwargs, &litellm_params)?;
    let call_id = optional_string(request.kwargs, "litellm_call_id")?;
    let resolved = resolved_model(request, &litellm_params, resolution)?;
    if litellm_params.custom_llm_provider.is_none() {
        litellm_params.custom_llm_provider = Some(resolved.provider.slug().to_owned());
    }
    let call = ResolvedCall {
        model: resolved.model,
        provider: resolved.provider.slug().to_owned(),
        litellm_params,
        call_id,
    };
    match provider_config_for(&call.model, &resolved.provider, configs) {
        Some(provider_config) => native_setup(request, provider_config, call, logging),
        None => adapter_setup(request, call, logging),
    }
}

// string mock responses belong to the caller; only a falsy (absent or false) stream reaches the guard
fn reject_string_mock<E>(
    kwargs: &Object,
    litellm_params: &GenerateContentLiteLLMParams,
) -> Result<(), GenAiSetupError<E>>
where
    E: Error + 'static,
{
    let stream = match kwargs.get("stream") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(stream)) => *stream,
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(GenAiSetupError::RequestField(RequestFieldError {
                field: "stream",
                expected: "a boolean",
            }));
        }
    };
    if !stream && matches!(&litellm_params.mock_response, Some(Json::Text(_))) {
        return Err(GenAiSetupError::MockResponseHandledByCaller);
    }
    Ok(())
}

fn resolved_model<E>(
    request: &SetupRequest<'_>,
    litellm_params: &GenerateContentLiteLLMParams,
    resolution: &ResolutionContext<'_>,
) -> Result<ResolvedModel, GenAiSetupError<E>>
where
    E: Error + 'static,
{
    resolve(
        &ResolutionRequest {
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            api_base: litellm_params.api_base.as_deref(),
            api_key: litellm_params.api_key.as_ref(),
        },
        resolution,
    )
    .map_err(GenAiSetupError::Resolution)
}

fn native_setup<'call, 'configs, C>(
    request: &SetupRequest<'_>,
    provider_config: &'configs C,
    call: ResolvedCall,
    logging: Option<&'call mut LoggingState>,
) -> Result<GenerateContentSetupResult<'configs, 'call, C>, GenAiSetupError<C::Error>>
where
    C: GenerateContentProviderConfig,
{
    let absent_config = Object::new();
    let generation_config =
        provider_config.map_optional_params(request.config.unwrap_or(&absent_config), &call.model);
    let instruction = system_instruction(request.kwargs);
    let native_request_fields =
        native_request_fields(request.kwargs, provider_config.request_top_level_fields());
    let body = transformed_body(
        provider_config,
        request,
        &call.model,
        &generation_config,
        instruction.as_ref(),
    )?;
    let required_logging = logging.ok_or(GenAiSetupError::LoggingRequired)?;
    record_pre_call(required_logging, request.kwargs, &call, &generation_config)
        .map_err(GenAiSetupError::Logging)?;
    Ok(GenerateContentSetupResult {
        model: call.model,
        custom_llm_provider: call.provider,
        request_body: body,
        provider_config: Some(provider_config),
        config_dict: generation_config,
        native_request_fields,
        litellm_params: call.litellm_params,
        logging: required_logging,
        litellm_call_id: call.call_id,
    })
}

// Logging.update_from_kwargs receives the call identity and the optional params the native path mapped
fn record_pre_call(
    logging: &mut LoggingState,
    kwargs: &Object,
    call: &ResolvedCall,
    config_dict: &Object,
) -> Result<(), LoggingUpdateError> {
    log_pre_call(
        logging,
        kwargs,
        &CallLog {
            model: call.model.as_str(),
            config_dict,
            call_id: call.call_id.as_deref(),
            provider: call.provider.as_str(),
        },
    )
}

fn adapter_setup<'call, 'configs, C>(
    request: &SetupRequest<'_>,
    call: ResolvedCall,
    logging: Option<&'call mut LoggingState>,
) -> Result<GenerateContentSetupResult<'configs, 'call, C>, GenAiSetupError<C::Error>>
where
    C: GenerateContentProviderConfig,
{
    let required_logging = logging.ok_or(GenAiSetupError::LoggingRequired)?;
    Ok(GenerateContentSetupResult {
        model: call.model,
        custom_llm_provider: call.provider,
        request_body: Json::Object(Object::new()),
        provider_config: None,
        config_dict: request.config.cloned().unwrap_or_default(),
        native_request_fields: Object::new(),
        litellm_params: call.litellm_params,
        logging: required_logging,
        litellm_call_id: call.call_id,
    })
}

fn transformed_body<C>(
    provider_config: &C,
    request: &SetupRequest<'_>,
    model: &str,
    generation_config: &Object,
    system_instruction: Option<&Json>,
) -> Result<Json, GenAiSetupError<C::Error>>
where
    C: GenerateContentProviderConfig,
{
    provider_config
        .transform_request(&GenerateContentRequest {
            model,
            contents: request.contents,
            tools: request.tools,
            generation_config,
            system_instruction,
        })
        .map_err(GenAiSetupError::Transform)
}

// only the fields the provider config names, and only when kwargs carries them
fn native_request_fields(kwargs: &Object, fields: &'static [&'static str]) -> Object {
    let mut extracted = Object::new();
    for field in fields {
        if let Some(value) = kwargs.get(*field) {
            extracted.insert((*field).to_owned(), value.clone());
        }
    }
    extracted
}

// kwargs.get("systemInstruction") or kwargs.get("system_instruction"), with null read as absent
fn system_instruction(kwargs: &Object) -> Option<Json> {
    for name in ["systemInstruction", "system_instruction"] {
        match kwargs.get(name) {
            None | Some(Json::Null) => {}
            Some(value) => return Some(value.clone()),
        }
    }
    None
}
