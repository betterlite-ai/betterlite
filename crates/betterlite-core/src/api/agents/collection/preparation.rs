use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::interactions::agents_create_read::{CreateAgentRequest, ListAgentsRequest};
use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::agents::{AgentLoggingCall, get_agents_api_config, make_logging_obj};

use super::{
    AgentCreation, AgentListing, AgentPreparationError, CREATE_ASYNC_FLAG, CREATE_CALL_TYPE,
    DEFAULT_PROVIDER, LIST_ASYNC_FLAG, LIST_CALL_TYPE, PROVIDER_FIELD,
};

pub(super) struct PreparedCreation {
    name: String,
    params: Params,
    extra_headers: BTreeMap<String, String>,
    extra_body: Option<Params>,
    timeout: Option<Duration>,
    pub(super) is_async: bool,
}

impl PreparedCreation {
    pub(super) fn request(&self, request_timeout: Duration) -> CreateAgentRequest<'_> {
        CreateAgentRequest {
            name: &self.name,
            litellm_params: &self.params,
            extra_headers: self.extra_headers.clone(),
            extra_body: self.extra_body.clone(),
            timeout: self.timeout,
            request_timeout,
        }
    }
}

pub(super) struct PreparedListing {
    params: Params,
    extra_headers: BTreeMap<String, String>,
    timeout: Option<Duration>,
    pub(super) is_async: bool,
}

impl PreparedListing {
    pub(super) fn request(&self) -> ListAgentsRequest<'_> {
        ListAgentsRequest {
            litellm_params: &self.params,
            extra_headers: self.extra_headers.clone(),
            timeout: self.timeout,
        }
    }
}

pub(super) fn prepare_creation(
    logging: &mut LoggingState,
    call: &AgentCreation<'_>,
    provider: &str,
) -> Result<PreparedCreation, AgentPreparationError> {
    // LiteLLM pops the flag before it builds the params and the logging metadata
    let is_async = matches!(call.kwargs.get(CREATE_ASYNC_FLAG), Some(Json::Bool(true)));
    let kwargs = without_flag(call.kwargs, CREATE_ASYNC_FLAG);
    let mut params = params_of(&kwargs);
    insert_text(&mut params, "base_agent", call.base_agent);
    insert_text(&mut params, "instructions", call.instructions);
    if let Some(environment) = call.base_environment {
        params.insert("base_environment".to_owned(), json_value(environment));
    }
    params
        .entry(PROVIDER_FIELD.to_owned())
        .or_insert_with(|| JsonValue::Text(provider.to_owned()));
    make_logging_obj(
        logging,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: call.name,
            custom_llm_provider: provider,
            call_type: CREATE_CALL_TYPE,
            optional_params: IndexMap::new(),
        },
    )?;
    // the provider still has to have a native agents API, whatever config the caller handed in
    let _provider = get_agents_api_config(provider)?;
    Ok(PreparedCreation {
        name: call.name.to_owned(),
        params,
        extra_headers: call.extra_headers.clone(),
        extra_body: call.extra_body.map(params_of),
        timeout: call.timeout,
        is_async,
    })
}

pub(super) fn prepare_listing(
    logging: &mut LoggingState,
    call: &AgentListing<'_>,
    provider: &str,
) -> Result<PreparedListing, AgentPreparationError> {
    let is_async = matches!(call.kwargs.get(LIST_ASYNC_FLAG), Some(Json::Bool(true)));
    let kwargs = without_flag(call.kwargs, LIST_ASYNC_FLAG);
    let mut params = params_of(&kwargs);
    params
        .entry(PROVIDER_FIELD.to_owned())
        .or_insert_with(|| JsonValue::Text(provider.to_owned()));
    make_logging_obj(
        logging,
        AgentLoggingCall {
            kwargs: &kwargs,
            model: "",
            custom_llm_provider: provider,
            call_type: LIST_CALL_TYPE,
            optional_params: IndexMap::new(),
        },
    )?;
    // the provider still has to have a native agents API, whatever config the caller handed in
    let _provider = get_agents_api_config(provider)?;
    Ok(PreparedListing {
        params,
        extra_headers: call.extra_headers.clone(),
        timeout: call.timeout,
        is_async,
    })
}

// LiteLLM's `or` chain defaults an absent argument, and docs/compatibility keeps an explicit empty string as sent
pub(super) fn provider_of<'values>(
    argument: Option<&'values str>,
    kwargs: &'values IndexMap<String, Json>,
) -> Result<&'values str, AgentPreparationError> {
    if let Some(name) = argument {
        return Ok(name);
    }
    match kwargs.get(PROVIDER_FIELD) {
        None | Some(Json::Null) => Ok(DEFAULT_PROVIDER),
        Some(Json::Text(name)) => Ok(name),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentPreparationError::FieldType {
                field: PROVIDER_FIELD,
            })
        }
    }
}

// LiteLLM's kwargs.pop(name, False): the operation's own flag never reaches the params a provider sees
fn without_flag(kwargs: &IndexMap<String, Json>, name: &str) -> IndexMap<String, Json> {
    let mut kept = kwargs.clone();
    kept.shift_remove(name);
    kept
}

fn params_of(kwargs: &IndexMap<String, Json>) -> Params {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), json_value(field)))
                .collect(),
        ),
    }
}

fn insert_text(params: &mut Params, name: &str, value: Option<&str>) {
    if let Some(text) = value {
        params.insert(name.to_owned(), JsonValue::Text(text.to_owned()));
    }
}
