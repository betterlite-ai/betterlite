pub mod blocking;

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::{
    CallParams, ParamValue, pre_call_record, read_params, request_headers,
};
use betterlite_openai::containers::container_file_http::response::raise_for_error_status;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    ContainerMethod, Selected, TransferError,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::json;

use super::error::FileListHandlerError;

// One file-list call as the two handlers take it: the resolved provider config, the params the call was prepared with, and the client the caller may have supplied
pub struct ContainerFileListCall<'call, Source: ClientSource> {
    pub config: &'call dyn BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    pub container_id: &'call str,
    pub after: Option<&'call str>,
    pub limit: Option<i64>,
    pub order: Option<&'call str>,
    // LiteLLM accepts the timeout and never sends it, so the selected client's own deadline stays
    pub timeout_seconds: f64,
    // a supplied client is kept only when it is the kind this path sends with
    pub blocking_client: Option<&'call Source::Blocking>,
    pub async_client: Option<&'call Source::Asynchronous>,
}

// ast-grep-ignore: no-long-names
pub async fn async_container_file_list_handler<Source, Hook>(
    clients: &Source,
    hook: &Hook,
    call: &ContainerFileListCall<'_, Source>,
) -> Result<ContainerFileListResponse, FileListHandlerError>
where
    Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>,
    Hook: PreCallHook,
{
    let params = read_params(call.litellm_params)?;
    let client = Selected::choose(call.async_client, || {
        clients.async_client(params.ssl_verify.as_ref())
    })?;
    let request = prepare_request(hook, call, &params)?;
    let reply = client
        .get()
        .send(&request)
        .await
        .map_err(|error| transport_error(call.config, &error))?;
    finish(call.config, &reply)
}

// validate_environment, get_complete_url, the provider transform, pre_call, then the GET itself
pub(crate) fn prepare_request<Source, Hook>(
    hook: &Hook,
    call: &ContainerFileListCall<'_, Source>,
    params: &CallParams<'_>,
) -> Result<ContainerHttpRequest, FileListHandlerError>
where
    Source: ClientSource,
    Hook: PreCallHook,
{
    let headers = request_headers(call.config, call.extra_headers, params.api_key.as_ref());
    let api_base = call
        .config
        .get_complete_url(params.api_base, call.litellm_params)?;
    let extra_query = extra_query_object(call.extra_query);
    let query = ContainerListQuery {
        after: call.after,
        limit: call.limit,
        order: call.order,
        extra_query: extra_query.as_ref(),
    };
    let context = ContainerRequestContext {
        litellm_params: call.litellm_params,
        headers: &headers,
    };
    let (url, sent) = call.config.transform_container_file_list_request(
        call.container_id,
        &api_base,
        context,
        &query,
    )?;
    let pairs = query_pairs(&sent)?;
    log_pre_call(hook, call, &url, &headers, &pairs)?;
    Ok(ContainerHttpRequest {
        method: ContainerMethod::Get,
        url,
        headers,
        query: pairs,
        file: None,
    })
}

// GenericLiteLLMParams already carries the caller's extra_query as typed JSON values
pub(super) fn extra_query_object(query: Option<&IndexMap<String, ParamValue>>) -> Option<Object> {
    let values = query?;
    let mut object = Object::new();
    for (name, value) in values {
        let encoded = match value {
            ParamValue::Text(text) => json!(text),
            ParamValue::Integer(number) => json!(number),
        };
        object.insert(name.clone(), encoded);
    }
    Some(object)
}

// The transform answers a JSON object of params; httpx renders a null as "", a list as one pair per item, and refuses anything deeper
pub(super) fn query_pairs(params: &Object) -> Result<Vec<(String, String)>, FileListHandlerError> {
    let encoded = serde_json::to_string(params)?;
    let values: IndexMap<String, JsonValue> = serde_json::from_str(&encoded)?;
    let mut pairs = Vec::with_capacity(values.len());
    for (name, value) in &values {
        push_query_pairs(name, value, &mut pairs)?;
    }
    Ok(pairs)
}

fn push_query_pairs(
    name: &str,
    value: &JsonValue,
    pairs: &mut Vec<(String, String)>,
) -> Result<(), FileListHandlerError> {
    if let JsonValue::Array(items) = value {
        for item in items {
            pairs.push((name.to_owned(), scalar_text(name, item)?));
        }
        return Ok(());
    }
    pairs.push((name.to_owned(), scalar_text(name, value)?));
    Ok(())
}

fn scalar_text(name: &str, value: &JsonValue) -> Result<String, FileListHandlerError> {
    match value {
        JsonValue::Null => Ok(String::new()),
        JsonValue::Text(text) => Ok(text.clone()),
        JsonValue::Bool(flag) => Ok(flag.to_string()),
        JsonValue::Number(number) => Ok(number.to_string()),
        JsonValue::Array(_) | JsonValue::Object(_) => Err(FileListHandlerError::QueryValue {
            field: name.to_owned(),
        }),
    }
}

// pre_call(input="", api_key="", additional_args={"api_base": url, "headers": headers, "params": params})
fn log_pre_call<Source, Hook>(
    hook: &Hook,
    call: &ContainerFileListCall<'_, Source>,
    url: &str,
    headers: &BTreeMap<String, String>,
    pairs: &[(String, String)],
) -> Result<(), FileListHandlerError>
where
    Source: ClientSource,
    Hook: PreCallHook,
{
    let logged: IndexMap<String, String> = pairs.iter().cloned().collect();
    let record = pre_call_record(url, headers, &logged);
    let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
    let outcome = hook.pre_call(&mut state, &record);
    drop(state);
    outcome?;
    Ok(())
}

// raise_for_error_status, then the provider's response transform
pub(crate) fn finish(
    config: &dyn BaseContainerConfig,
    reply: &ContainerHttpReply,
) -> Result<ContainerFileListResponse, FileListHandlerError> {
    raise_for_error_status(config, reply)?;
    config
        .transform_container_file_list_response(&reply.body)
        .map_err(FileListHandlerError::Config)
}

// _handle_error: a transport failure has no provider status, and LiteLLM's read timeout becomes a 408 Timeout
pub(crate) fn transport_error(
    config: &dyn BaseContainerConfig,
    error: &TransferError,
) -> FileListHandlerError {
    let status = match error {
        TransferError::Transport(TransportError::Deadline(_)) => 408,
        TransferError::Transport(_)
        | TransferError::Url { .. }
        | TransferError::HeaderName { .. } => 500,
    };
    FileListHandlerError::Transport(config.get_error_class(
        &error.to_string(),
        status,
        &BTreeMap::new(),
    ))
}
