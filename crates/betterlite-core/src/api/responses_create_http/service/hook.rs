use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call as log_pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use indexmap::IndexMap;

use crate::api::responses_hook_policy::callbacks::{
    CallbackLists, DeploymentHookDispatcher, NamedCallbackResolver, ResponsesHookRequest,
    run_responses_pre_hook,
};

use super::super::config::{CreateParams, Headers, Params};
use super::super::convert::{ValueKind as _, round_trip};
use super::super::error::{CreateError, ParamError};
use super::{CreateCall, LogContext};

// The five values the sync pre-hook may replace, owned so the rest of the handler borrows one shape
pub(super) struct Applied {
    pub(super) model: String,
    pub(super) input: ResponsesInput,
    pub(super) custom_llm_provider: String,
    pub(super) optional_params: Object,
    pub(super) params: CreateParams,
}

impl Applied {
    pub(super) fn original(call: &CreateCall<'_>) -> Self {
        Self {
            model: call.model.to_owned(),
            input: call.input.clone(),
            custom_llm_provider: call.custom_llm_provider.to_owned(),
            optional_params: call.optional_params.clone(),
            params: call.params.clone(),
        }
    }
}

// _run_sync_responses_pre_call_deployment_hook; only the sync handler runs it
pub(super) async fn apply<D>(
    call: &CreateCall<'_>,
    callbacks: &CallbackLists<'_>,
    resolver: &dyn NamedCallbackResolver,
    dispatcher: &D,
) -> Result<Applied, CreateError>
where
    D: DeploymentHookDispatcher + ?Sized,
{
    let request = ResponsesHookRequest {
        model: call.model.to_owned(),
        input: round_trip(call.input)?,
        custom_llm_provider: call.custom_llm_provider.to_owned(),
        optional_params: round_trip(call.optional_params)?,
        generic_params: call.params.to_map(),
    };
    let outcome = run_responses_pre_hook(request, callbacks, resolver, dispatcher).await?;
    let input_text = serde_json::to_string(&outcome.input).map_err(CreateError::Encode)?;
    let input = serde_json::from_str(&input_text)
        .map_err(|source| CreateError::Params(ParamError::HookInput(source)))?;
    Ok(Applied {
        model: outcome.model,
        input,
        custom_llm_provider: outcome.custom_llm_provider,
        optional_params: round_trip(&outcome.optional_params)?,
        params: CreateParams::from_map(outcome.generic_params)?,
    })
}

// request_context={input, ...optional_params, litellm_params}; the optional update cannot fail for a declared map
pub(super) fn request_context(
    input: &ResponsesInput,
    optional_params: &Object,
    params: &Params,
) -> Result<Params, CreateError> {
    let mut context: Params = Params::new();
    context.insert(String::from("input"), round_trip(input)?);
    for (key, value) in optional_params {
        context.insert(key.clone(), round_trip(value)?);
    }
    context.insert(String::from("litellm_params"), Json::Object(params.clone()));
    Ok(context)
}

// The extra_headers entry of the optional map reaches validate_environment before the separate argument overlays it
pub(super) fn optional_headers(optional_params: &Object) -> Result<Headers, CreateError> {
    let Some(value) = optional_params.get("extra_headers") else {
        return Ok(Headers::new());
    };
    let fields = match value {
        PromptJson::Null => return Ok(Headers::new()),
        PromptJson::Object(fields) => fields,
        other @ (PromptJson::Bool(_)
        | PromptJson::Number(_)
        | PromptJson::Text(_)
        | PromptJson::List(_)) => {
            return Err(CreateError::OptionalHeaders {
                found: other.value_kind(),
            });
        }
    };
    let mut headers = Headers::new();
    for (name, entry) in fields {
        let text = match entry {
            PromptJson::Text(text) => text,
            other @ (PromptJson::Null
            | PromptJson::Bool(_)
            | PromptJson::Number(_)
            | PromptJson::List(_)
            | PromptJson::Object(_)) => {
                return Err(CreateError::OptionalHeader {
                    name: name.clone(),
                    found: other.value_kind(),
                });
            }
        };
        headers.insert(name.clone(), text.clone());
    }
    Ok(headers)
}

// The pre-call a production request runs: the original input, no api key, the final body, base URL and signed headers
pub(super) fn pre_call(
    input: &ResponsesInput,
    url: &str,
    headers: &Headers,
    data: &Object,
    log: &mut LogContext<'_, '_>,
) -> Result<(), CreateError> {
    let mut logged_headers = IndexMap::new();
    for (name, value) in headers {
        logged_headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: logged_input(input),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.to_owned()),
            headers: Some(logged_headers),
            complete_input_dict: Some(RawBody::Object(logged_object(data))),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    log_pre_call(log.state, &record, log.env).map_err(CreateError::Logging)?;
    Ok(())
}

fn logged_input(input: &ResponsesInput) -> Json {
    match input {
        ResponsesInput::Text(text) => Json::Text(text.clone()),
        ResponsesInput::Items(items) => {
            Json::List(items.iter().map(|item| logged_json(&item.0)).collect())
        }
    }
}

fn logged_object(data: &Object) -> IndexMap<String, Json> {
    data.iter()
        .map(|(name, value)| (name.clone(), logged_json(value)))
        .collect()
}

fn logged_json(value: &PromptJson) -> Json {
    match value {
        PromptJson::Null => Json::Null,
        PromptJson::Bool(flag) => Json::Bool(*flag),
        PromptJson::Number(number) => Json::Number(number.clone()),
        PromptJson::Text(text) => Json::Text(text.clone()),
        PromptJson::List(items) => Json::List(items.iter().map(logged_json).collect()),
        PromptJson::Object(fields) => Json::Object(logged_object(fields)),
    }
}
