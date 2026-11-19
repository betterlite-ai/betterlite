use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::response_resources::request::ResourceRequest;
use betterlite_prompts::arguments::json::Object as ResourceObject;
use indexmap::IndexMap;

use super::super::config::ResponsesConfig;
use super::super::error::WriteError;
use super::{
    ClientKind, ClientSource, LogContext, Operation, Selected, SharedSession, WriteDeps,
    WriteTarget,
};

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    operation: Operation,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, WriteError>
where
    'deps: 'borrow,
    C: ResponsesConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    let shared_session = match mode {
        ClientKind::Sync => None,
        ClientKind::Asynchronous => deps.shared_session,
    };
    if mode == ClientKind::Asynchronous {
        tracing::debug!(
            "Creating HTTP client for {} with shared_session: {}",
            operation.client_label(),
            session_label(shared_session)
        );
    }
    Ok(Selected::Created(
        deps.source.client(ssl_verify, shared_session)?,
    ))
}

const fn session_label(shared: Option<SharedSession>) -> &'static str {
    match shared {
        Some(SharedSession) => "Some",
        None => "None",
    }
}

// B508's validate_environment rewrites Authorization in the map it is handed, so the caller's entries go in first
pub(super) fn validated_headers<C>(config: &C, target: &WriteTarget<'_>) -> BTreeMap<String, String>
where
    C: ResponsesConfig,
{
    let caller = target.extra_headers.cloned().unwrap_or_default();
    config.validate_environment(caller, target.params)
}

pub(super) fn write_endpoint<C>(
    config: &C,
    target: &WriteTarget<'_>,
    operation: Operation,
) -> Result<(String, ResourceRequest), WriteError>
where
    C: ResponsesConfig,
{
    let api_base = config
        .complete_url(target.params)
        .map_err(WriteError::Url)?;
    let transformed = match operation {
        Operation::Delete => config.delete_request(target.response_id, &api_base),
        Operation::Cancel => config.cancel_request(target.response_id, &api_base),
    };
    let resource = transformed.map_err(WriteError::Resource)?;
    Ok((api_base, resource))
}

// The pre-call record and the wire body both read B509's payload, whose JSON type differs from the logging one
pub(super) fn resource_payload(
    payload: &ResourceObject,
) -> Result<IndexMap<String, Json>, WriteError> {
    let encoded = serde_json::to_value(payload).map_err(WriteError::Encode)?;
    serde_json::from_value(encoded).map_err(WriteError::Encode)
}

pub(super) struct LoggedCall<'call> {
    pub(super) operation: Operation,
    pub(super) response_id: &'call str,
    pub(super) api_base: &'call str,
    pub(super) url: &'call str,
    pub(super) headers: &'call BTreeMap<String, String>,
    pub(super) payload: &'call IndexMap<String, Json>,
}

pub(super) fn log_pre_call(
    call: &LoggedCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), WriteError> {
    let headers: IndexMap<String, Json> = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    // The declared deviation: LiteLLM's delete logs the built-in input function, which has no request-data form
    let input = match call.operation {
        Operation::Delete => String::new(),
        Operation::Cancel => call.response_id.to_owned(),
    };
    // LiteLLM's cancel handler logs the suffixed URL; delete logs the unsuffixed base
    let logged_base = match call.operation {
        Operation::Delete => call.api_base,
        Operation::Cancel => call.url,
    };
    let record = PreCall {
        input: Json::Text(input),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(logged_base.to_owned()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(call.payload.clone())),
            request_str: None,
            openai_sdk: None,
            // B249 keeps no api_key, so LiteLLM's empty one has no field here
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}
