use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::ResolutionError;

use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::preparation::response_text;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::normalize_openai_chat_model;
use crate::api::responses_provider_selection::resolution::{ModelResolver, resolve_model_provider};

use super::deps::AsyncCreateDeps;
use super::error::{AsyncCreateCause, AsyncCreateError};
use super::hook::prompted;

// The sync call aresponses hands on: converted text, the trusted async flag, the merged prompt and the provider it resolved
pub(super) struct PreparedCall {
    pub(super) call: ResponsesCreateCall,
    pub(super) provider: String,
    pub(super) model: String,
}

pub(super) async fn prepare(
    deps: &AsyncCreateDeps<'_>,
    call: &ResponsesCreateCall,
) -> Result<PreparedCall, AsyncCreateError> {
    let mut converted = convert(call)?;
    let provider = initial_provider(deps.resolver, call).map_err(|error| AsyncCreateError {
        cause: Box::new(AsyncCreateCause::ProviderResolution(error)),
        provider: String::new(),
        model: call.model.clone(),
    })?;
    converted.custom_llm_provider = Some(provider.clone());
    let Some(port) = deps.prompts else {
        return Ok(PreparedCall {
            call: converted,
            provider,
            model: call.model.clone(),
        });
    };
    prompted(deps, call, converted, provider, port).await
}

// LiteLLM converts text_format to text, then sends aresponses=True on to the sync entry
fn convert(call: &ResponsesCreateCall) -> Result<ResponsesCreateCall, AsyncCreateError> {
    let mut converted = call.clone();
    let text = response_text(call).map_err(|error| AsyncCreateError {
        cause: Box::new(AsyncCreateCause::Create(error)),
        provider: String::new(),
        model: call.model.clone(),
    })?;
    converted.text = text;
    converted.text_format = None;
    converted
        .kwargs
        .insert(String::from("aresponses"), JsonValue::Bool(true));
    Ok(converted)
}

// LiteLLM reads local_vars["base_url"], which aresponses never has, so detection gets no api_base
fn initial_provider(
    resolver: &dyn ModelResolver,
    call: &ResponsesCreateCall,
) -> Result<String, ResolutionError> {
    if let Some(named) = &call.custom_llm_provider {
        return Ok(named.clone());
    }
    let (normalized, _prefixed) = normalize_openai_chat_model(&call.model);
    let mut params = CallParams::default();
    let mut local_vars = ParameterMap::new();
    let (_model, provider) =
        resolve_model_provider(resolver, &normalized, None, &mut params, &mut local_vars)?;
    Ok(provider)
}
