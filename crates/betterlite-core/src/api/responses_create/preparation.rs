use super::call::{
    ResponsesCreateCall, input_value, named_locals, resolution_params, session_metadata, text_param,
};
use super::deps::PromptSource;
use super::error::CreateError;
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_json::{decision_json, json_value};
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
// A bool in the create state; the newtype keeps struct_excessive_bools off the records that declare four
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateFlag(pub bool);
pub struct Prepared<'call> {
    pub call: &'call ResponsesCreateCall,
    pub kwargs: ParameterMap,
    pub local_vars: ParameterMap,
    pub input: ResponsesInput,
    pub model: String,
    pub provider: Option<String>,
    pub use_chat: CreateFlag,
    pub extra_headers: Option<Headers>,
    pub text: Option<Json>,
    pub is_async: CreateFlag,
    pub skip_mcp: CreateFlag,
    pub require_encrypted: CreateFlag,
    pub deployment_model_info: Option<Json>,
}
pub struct Resolved {
    pub model: String,
    pub provider: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub deployment_model_info: Json,
}
struct Flags {
    is_async: CreateFlag,
    skip_mcp: CreateFlag,
    require_encrypted: CreateFlag,
    use_chat: CreateFlag,
}
pub fn begin<'call>(
    call: &'call ResponsesCreateCall,
    resolver: &dyn ModelResolver,
    lookup: &dyn ConfigLookup,
    prompts: Option<&dyn PromptSource>,
) -> Result<Prepared<'call>, CreateError> {
    let mut kwargs = connection_kwargs(call);
    let flags = request_flags(&mut kwargs);
    let extra_headers = merged_headers(call, &kwargs)?;
    let text = response_text(call)?;
    let (model, provider, use_chat) = resolved_names(call, resolver, flags.use_chat.0)?;
    let mut local_vars: ParameterMap = named_locals(call, text.as_ref()).into_iter().collect();
    // the closure's return type upcasts the Send port to the manager the prompting pipeline takes
    let manager = prompts.map(|source: &dyn PromptSource| -> &dyn PromptManager { source });
    let outcome = apply_prompt_management(
        PromptRequest {
            input: input_value(&call.input),
            model: &model,
            custom_llm_provider: provider.as_deref(),
            use_chat_completions_api: use_chat,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        manager,
        lookup,
        resolver,
    )?;
    let input: ResponsesInput = transcode(&outcome.input)?;
    Ok(Prepared {
        call,
        kwargs,
        local_vars,
        input,
        model: outcome.model,
        provider: outcome.custom_llm_provider,
        use_chat: CreateFlag(use_chat),
        extra_headers,
        text,
        is_async: flags.is_async,
        skip_mcp: flags.skip_mcp,
        require_encrypted: flags.require_encrypted,
        deployment_model_info: outcome.deployment_model_info.as_ref().map(decision_json),
    })
}
fn request_flags(kwargs: &mut ParameterMap) -> Flags {
    let is_async = kwargs.shift_remove("aresponses") == Some(JsonValue::Bool(true));
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
    let require_encrypted =
        kwargs.shift_remove("_require_encrypted_task_support") == Some(JsonValue::Bool(true));
    let use_chat = pop_use_chat_completions_api(kwargs);
    Flags {
        is_async: CreateFlag(is_async),
        skip_mcp: CreateFlag(skip_mcp),
        require_encrypted: CreateFlag(require_encrypted),
        use_chat: CreateFlag(use_chat),
    }
}
pub(crate) fn response_text(call: &ResponsesCreateCall) -> Result<Option<Json>, CreateError> {
    let text_format = call
        .text_format
        .as_ref()
        .filter(|value| **value != Json::Null)
        .map(json_value);
    let given_text = call
        .text
        .as_ref()
        .filter(|value| **value != Json::Null)
        .map(json_value);
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
fn resolved_names(
    call: &ResponsesCreateCall,
    resolver: &dyn ModelResolver,
    mut use_chat: bool,
) -> Result<(String, Option<String>, bool), CreateError> {
    let (normalized, prefixed) = normalize_openai_chat_model(&call.model);
    use_chat |= prefixed;
    let (model, provider) = if let Some(named) = call.custom_llm_provider.as_deref() {
        (normalized, Some(named.to_owned()))
    } else {
        let mut params = CallParams::default();
        let mut local_vars = ParameterMap::new();
        let (resolved_model, named_provider) =
            resolve_model_provider(resolver, &normalized, None, &mut params, &mut local_vars)?;
        (resolved_model, Some(named_provider))
    };
    Ok((model, provider, use_chat))
}
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
    prep.model.clone_from(&model);
    prep.provider = Some(provider.clone());
    Ok(Resolved {
        model,
        provider,
        api_base: params.api_base,
        api_key: params.api_key,
        deployment_model_info: prep.deployment_model_info.clone().unwrap_or(Json::Null),
    })
}
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
    let given = prep.extra_headers.clone().unwrap_or_default();
    let headers: IndexMap<String, String> = given.into_iter().collect();
    let updated = add_provider_affinity_header(&headers, &params)?;
    prep.extra_headers = Some(updated.into_iter().collect());
    Ok(())
}
pub(crate) fn connection_kwargs(call: &ResponsesCreateCall) -> ParameterMap {
    let mut kwargs = call.kwargs.clone();
    if let Some(base) = &call.api_base {
        kwargs.insert(String::from("api_base"), JsonValue::Text(base.clone()));
    }
    if let Some(key) = &call.api_key {
        let secret = JsonValue::Text(key.expose_secret().to_owned());
        kwargs.insert(String::from("api_key"), secret);
    }
    if let Some(verify) = call.ssl_verify {
        kwargs.insert(String::from("ssl_verify"), JsonValue::Bool(verify));
    }
    kwargs
}
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
    let Some(headers) = kwargs
        .get("headers")
        .filter(|value| **value != JsonValue::Null)
    else {
        return Ok(None);
    };
    let JsonValue::Object(fields) = headers else {
        return Err(CreateError::Field {
            field: "headers",
            expected: "an object",
        });
    };
    header_strings(fields.clone().into_iter()).map(Some)
}
fn header_strings<I>(pairs: I) -> Result<Headers, CreateError>
where
    I: Iterator<Item = (String, JsonValue)>,
{
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
