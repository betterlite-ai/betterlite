use super::call::{
    ResponsesCreateCall, input_value, named_locals, resolution_params, session_metadata, text_param,
};
use super::convert::{decision_json, json_value, reencode};
use super::error::CreateError;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_parameters::forwarded_headers::merge_forwarded_headers;
use crate::api::responses_parameters::text_format::text_format_to_text;
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prefixes::{
    normalize_openai_chat_model, pop_use_chat_completions_api,
};
use crate::api::responses_provider_selection::prompting::{
    PromptManager, PromptRequest, apply_prompt_management,
};
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::{ModelResolver, resolve_model_provider};
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::{AffinityParams, add_provider_affinity_header};
use betterlite_openai::response_input::ResponsesInput;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
// Everything the create wrapper computes before a provider is chosen
pub struct Prepared<'call> {
    pub call: &'call ResponsesCreateCall,
    pub kwargs: ParameterMap,
    pub local_vars: ParameterMap,
    pub input: ResponsesInput,
    pub model: String,
    pub provider: Option<String>,
    pub use_chat: bool,
    pub extra_headers: Option<Headers>,
    pub text: Option<Json>,
    pub is_async: bool,
    pub skip_mcp: bool,
    pub require_encrypted: bool,
    pub deployment_model_info: Option<Json>,
}
// get_llm_provider's answer once the caller's key and base are folded in
pub struct Resolved {
    pub model: String,
    pub provider: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub deployment_model_info: Json,
}
// The wrapper's head: the trusted pops, the header merge, the text conversion, the model prefix and the provisional provider
pub fn begin<'call>(
    call: &'call ResponsesCreateCall,
    resolver: &dyn ModelResolver,
    lookup: &dyn ConfigLookup,
    prompts: Option<&dyn PromptManager>,
) -> Result<Prepared<'call>, CreateError> {
    let mut kwargs = connection_kwargs(call);
    let is_async = matches!(
        kwargs.shift_remove("aresponses"),
        Some(JsonValue::Bool(true))
    );
    let skip_mcp = kwargs
        .shift_remove("_skip_mcp_handler")
        .is_some_and(|value| match value {
            JsonValue::Null => false,
            JsonValue::Bool(flag) => flag,
            JsonValue::Number(number) => number.as_f64().is_some_and(|float| float != 0.0_f64),
            JsonValue::Text(text) => !text.is_empty(),
            JsonValue::Array(items) => !items.is_empty(),
            JsonValue::Object(fields) => !fields.is_empty(),
        });
    let require_encrypted = matches!(
        kwargs.shift_remove("_require_encrypted_task_support"),
        Some(JsonValue::Bool(true))
    );
    let mut use_chat = pop_use_chat_completions_api(&mut kwargs);
    let extra_headers = merged_headers(call, &kwargs)?;
    let text_format = call.text_format.as_ref().map(json_value);
    let given_text = call.text.as_ref().map(json_value);
    let text = text_format_to_text(text_format.as_ref(), given_text.as_ref())?
        .as_ref()
        .map(decision_json);
    let (model, prefixed) = normalize_openai_chat_model(&call.model);
    use_chat |= prefixed;
    // get_llm_provider runs before the provider config exists, with the caller's key and base left out
    let (model, provider) = if let Some(named) = call.custom_llm_provider.as_deref() {
        (model, Some(named.to_owned()))
    } else {
        let mut params = CallParams::default();
        let mut local_vars = ParameterMap::new();
        let (resolved, provider) =
            resolve_model_provider(resolver, &model, None, &mut params, &mut local_vars)?;
        (resolved, Some(provider))
    };
    let mut local_vars = named_locals(call, text.as_ref());
    let outcome = apply_prompt_management(
        PromptRequest {
            input: input_value(&call.input),
            model: &model,
            custom_llm_provider: provider.as_deref(),
            use_chat_completions_api: use_chat,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        prompts,
        lookup,
        resolver,
    )?;
    let input: ResponsesInput = reencode(&outcome.input)?;
    Ok(Prepared {
        call,
        kwargs,
        local_vars,
        input,
        model: outcome.model,
        provider: outcome.custom_llm_provider,
        use_chat,
        extra_headers,
        text,
        is_async,
        skip_mcp,
        require_encrypted,
        deployment_model_info: outcome.deployment_model_info,
    })
}
// The full resolution reads the caller's key and base, then lets the resolver override them
pub fn resolve(
    prep: &mut Prepared<'_>,
    call: &ResponsesCreateCall,
    resolver: &dyn ModelResolver,
) -> Result<Resolved, CreateError> {
    let mut params = resolution_params(call);
    let (model, provider) = resolve_model_provider(
        resolver,
        &prep.model,
        prep.provider.as_deref(),
        &mut params,
        &mut prep.local_vars,
    )?;
    if provider.is_empty() {
        return Err(CreateError::MissingProvider);
    }
    prep.model = model.clone();
    prep.provider = Some(provider.clone());
    Ok(Resolved {
        model,
        provider,
        api_base: params.api_base,
        api_key: params.api_key,
        deployment_model_info: prep.deployment_model_info.clone().unwrap_or(Json::Null),
    })
}
// add_provider_affinity_header only runs when the caller configured a header; otherwise the merged headers stand
pub fn affinity(prep: &mut Prepared<'_>, call: &ResponsesCreateCall) -> Result<(), CreateError> {
    let Some(header) = text_param(&prep.kwargs, "provider_affinity_header")? else {
        return Ok(());
    };
    let litellm_metadata = prep.kwargs.get("litellm_metadata").map(decision_json);
    let params = AffinityParams {
        provider_affinity_header: Some(header),
        session_id: text_param(&prep.kwargs, "session_id")?,
        litellm_session_id: text_param(&prep.kwargs, "litellm_session_id")?,
        metadata: session_metadata(call.metadata.as_ref(), "metadata")?,
        litellm_metadata: session_metadata(litellm_metadata.as_ref(), "litellm_metadata")?,
    };
    let headers: IndexMap<String, String> = prep
        .extra_headers
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let updated = add_provider_affinity_header(&headers, &params)?;
    prep.extra_headers = Some(updated.into_iter().collect());
    Ok(())
}
// dict(litellm_params): the caller's kwargs, with the connection values responses() names itself
pub(super) fn connection_kwargs(call: &ResponsesCreateCall) -> ParameterMap {
    let mut kwargs = call.kwargs.clone();
    if let Some(base) = &call.api_base {
        kwargs.insert(String::from("api_base"), JsonValue::Text(base.clone()));
    }
    if let Some(key) = &call.api_key {
        kwargs.insert(
            String::from("api_key"),
            JsonValue::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(verify) = call.ssl_verify {
        kwargs.insert(String::from("ssl_verify"), JsonValue::Bool(verify));
    }
    kwargs
}
// merge_client_forwarded_headers: the client's headers lose, case-insensitively, to the caller's extra headers
pub(super) fn merged_headers(
    call: &ResponsesCreateCall,
    kwargs: &ParameterMap,
) -> Result<Option<Headers>, CreateError> {
    let explicit = call.extra_headers.as_ref().map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
            .collect::<BTreeMap<String, JsonValue>>()
    });
    merge_forwarded_headers(explicit, client_headers(kwargs)?)
        .map(|headers| header_strings(headers.into_iter()))
        .transpose()
}
fn client_headers(kwargs: &ParameterMap) -> Result<Option<BTreeMap<String, String>>, CreateError> {
    match kwargs.get("headers") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(fields)) => header_strings(fields.clone().into_iter()).map(Some),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => Err(CreateError::Field {
            field: "headers",
            expected: "an object",
        }),
    }
}
fn header_strings(
    pairs: impl Iterator<Item = (String, JsonValue)>,
) -> Result<Headers, CreateError> {
    pairs
        .map(|(name, value)| match value {
            JsonValue::Text(text) => Ok((name, text)),
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_) => Err(CreateError::Field {
                field: "headers",
                expected: "an object of strings",
            }),
        })
        .collect()
}
