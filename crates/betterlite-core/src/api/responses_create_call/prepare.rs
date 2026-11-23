use indexmap::IndexMap;

use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_json::{decision_json, json_value};
use crate::api::responses_parameters::text_format::text_format_to_text;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::normalize_openai_chat_model;
use crate::api::responses_provider_selection::resolution::{ModelResolver, resolve_model_provider};

use super::deps::AsyncCreateDeps;
use super::error::AsyncCreateError;
use super::hook::prompted;

// The sync call aresponses hands on: converted text, the trusted async flag, the merged prompt and the provider it resolved
pub(super) struct PreparedCall {
    pub(super) call: ResponsesCreateCall,
    pub(super) provider: String,
    pub(super) model: String,
}

impl PreparedCall {
    #[must_use]
    pub(super) fn settled(call: ResponsesCreateCall, provider: String, model: &str) -> Self {
        Self {
            call,
            provider,
            model: model.to_owned(),
        }
    }
}

pub(super) async fn prepare(
    deps: &AsyncCreateDeps<'_>,
    call: &ResponsesCreateCall,
) -> Result<PreparedCall, AsyncCreateError> {
    let mut converted = convert(call)?;
    let provider = match initial_provider(deps.resolver, call) {
        Ok(provider) => provider,
        Err(error) => return Err(AsyncCreateError::create(error, "", &call.model)),
    };
    converted.custom_llm_provider = Some(provider.clone());
    let Some(port) = deps.prompts else {
        return Ok(PreparedCall::settled(converted, provider, &call.model));
    };
    prompted(deps, call, converted, provider, port).await
}

// LiteLLM converts text_format to text, then sends aresponses=True on to the sync entry
fn convert(call: &ResponsesCreateCall) -> Result<ResponsesCreateCall, AsyncCreateError> {
    let mut converted = call.clone();
    let text = match response_text(call) {
        Ok(text) => text,
        Err(error) => return Err(AsyncCreateError::create(error, "", &call.model)),
    };
    converted.text = text.map(decision_json);
    converted.text_format = None;
    converted
        .kwargs
        .insert(String::from("aresponses"), JsonValue::Bool(true));
    Ok(converted)
}

fn response_text(call: &ResponsesCreateCall) -> Result<Option<Json>, CreateError> {
    let text_format = call.text_format.as_ref().map(json_value);
    let given_text = call.text.as_ref().map(json_value);
    if given_text.is_none()
        && let Some(JsonValue::Object(fields)) = text_format.as_ref()
        && !matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind == "json_schema")
    {
        let format = JsonValue::Object(fields.clone());
        let envelope = IndexMap::from([(String::from("format"), format)]);
        return Ok(Some(decision_json(&JsonValue::Object(envelope))));
    }
    Ok(
        text_format_to_text(text_format.as_ref(), given_text.as_ref())?
            .as_ref()
            .map(decision_json),
    )
}

// LiteLLM reads local_vars["base_url"], which aresponses never has, so detection gets no api_base
fn initial_provider(
    resolver: &dyn ModelResolver,
    call: &ResponsesCreateCall,
) -> Result<String, CreateError> {
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
