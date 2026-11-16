use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::catalog::ContainerEndpoint;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::containers::container_file_http::ContainerFileReply;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::de::DeserializeOwned;
use serde_json::Number;
use serde_json::json;

use super::call::{CallSite, ContainerFileCall};
use super::error::ContainerFileOpError;
use super::handler::{ContainerConfigLookup, ContainerFileHandlerCall, ContainerIdDecoder};

// a mock answer, or everything the handler call needs
pub(super) enum Prepared<'deps> {
    Answered(ContainerFileReply),
    Ready(ReadyCall<'deps>),
}

pub(super) struct ReadyCall<'deps> {
    endpoint: &'deps str,
    config: &'deps dyn BaseContainerConfig,
    params: Object,
    values: BTreeMap<String, ParamValue>,
    timeout: Option<Duration>,
}

impl ReadyCall<'_> {
    pub(super) const fn handler_call<'call>(
        &'call self,
        call: &'call ContainerFileCall,
        logging: &'call Mutex<LoggingState>,
    ) -> ContainerFileHandlerCall<'call> {
        ContainerFileHandlerCall {
            endpoint_name: self.endpoint,
            config: self.config,
            params: &self.params,
            logging,
            extra_headers: call.extra_headers.as_ref(),
            extra_query: call.extra_query.as_ref(),
            timeout: self.timeout,
            values: &self.values,
            file: call.file.as_ref(),
        }
    }
}

// endpoint_factory.py's steps 1 to 6, shared by the blocking and async operations
pub(super) fn prepare<'deps>(
    endpoints: &'deps [ContainerEndpoint],
    site: &mut CallSite<'_>,
    decoder: &'deps dyn ContainerIdDecoder,
    configs: &'deps dyn ContainerConfigLookup,
) -> Result<Prepared<'deps>, ContainerFileOpError> {
    let entry = find_endpoint(endpoints, site.endpoint_name)?;
    if let Some(mock) = mock_body(site.call) {
        return Ok(Prepared::Answered(mock_reply(entry, mock)?));
    }
    let (container_id, params) = decode_managed_id(site, decoder)?;
    let provider = provider_id(&site.provider)?;
    let provider_config = configs.container_config(provider).ok_or_else(|| {
        ContainerFileOpError::NoProviderConfig {
            provider: site.provider.clone(),
        }
    })?;
    let values = handler_values(site.call, container_id.as_deref());
    let mut guard = site.logging.lock().unwrap_or_else(PoisonError::into_inner);
    log_pre_call(entry, site.call, &values, &site.provider, &mut guard)?;
    drop(guard);
    let timeout = resolved_timeout(site.call, site.default_timeout_seconds)?;
    Ok(Prepared::Ready(ReadyCall {
        endpoint: entry.name.as_str(),
        config: provider_config,
        params,
        values,
        timeout,
    }))
}

// a mock_response that is not null answers the call before any provider work
const fn mock_body(call: &ContainerFileCall) -> Option<&Json> {
    match &call.mock_response {
        Some(Json::Null) | None => None,
        Some(mock) => Some(mock),
    }
}

// step 3: the managed id decodes once, and its resolved provider replaces the caller's
fn decode_managed_id(
    site: &mut CallSite<'_>,
    decoder: &dyn ContainerIdDecoder,
) -> Result<(Option<String>, Object), ContainerFileOpError> {
    let mut params = call_params(site.call);
    let Some(managed) = site.call.container_id.as_deref() else {
        return Ok((None, params));
    };
    let decoded = decoder.decode(managed, &site.provider, &params)?;
    site.provider = decoded.custom_llm_provider;
    params = decoded.litellm_params;
    Ok((Some(decoded.container_id), params))
}

fn find_endpoint<'endpoints>(
    endpoints: &'endpoints [ContainerEndpoint],
    name: &str,
) -> Result<&'endpoints ContainerEndpoint, ContainerFileOpError> {
    endpoints
        .iter()
        .find(|entry| entry.name == name || entry.async_name == name)
        .ok_or_else(|| ContainerFileOpError::UnknownEndpoint {
            name: name.to_owned(),
        })
}

fn provider_id(provider: &str) -> Result<ProviderId, ContainerFileOpError> {
    let Ok(parsed) = provider.parse::<ProviderId>() else {
        return Err(ContainerFileOpError::UnknownProvider {
            provider: provider.to_owned(),
        });
    };
    Ok(parsed)
}

// GenericLiteLLMParams(**kwargs): every keyword, then the typed api_key, api_base and api_version
fn call_params(call: &ContainerFileCall) -> Object {
    let mut params = Object::new();
    for (name, value) in &call.extra {
        params.insert(name.clone(), json!(value));
    }
    if let Some(api_key) = &call.api_key {
        params.insert("api_key".to_owned(), json!(api_key.expose_secret()));
    }
    if let Some(api_base) = &call.api_base {
        params.insert("api_base".to_owned(), json!(api_base));
    }
    if let Some(api_version) = &call.api_version {
        params.insert("api_version".to_owned(), json!(api_version));
    }
    params
}

// the handler's kwargs: the catalog's values by name, with the decoded container id
fn handler_values(
    call: &ContainerFileCall,
    container_id: Option<&str>,
) -> BTreeMap<String, ParamValue> {
    let mut values = BTreeMap::new();
    if let Some(managed) = container_id {
        values.insert(
            "container_id".to_owned(),
            ParamValue::Text(managed.to_owned()),
        );
    }
    if let Some(file_id) = &call.file_id {
        values.insert("file_id".to_owned(), ParamValue::Text(file_id.clone()));
    }
    if let Some(after) = &call.after {
        values.insert("after".to_owned(), ParamValue::Text(after.clone()));
    }
    if let Some(limit) = call.limit {
        values.insert("limit".to_owned(), ParamValue::Integer(limit));
    }
    if let Some(order) = &call.order {
        values.insert("order".to_owned(), ParamValue::Text(order.clone()));
    }
    values
}

fn param_json(value: &ParamValue) -> Json {
    match value {
        ParamValue::Text(text) => Json::Text(text.clone()),
        ParamValue::Integer(number) => Json::Number(Number::from(*number)),
    }
}

// update_from_kwargs(kwargs, model="", optional_params=<path params>, litellm_params={"litellm_call_id": ...}, custom_llm_provider=...)
fn log_pre_call(
    endpoint: &ContainerEndpoint,
    call: &ContainerFileCall,
    values: &BTreeMap<String, ParamValue>,
    provider: &str,
    logging: &mut LoggingState,
) -> Result<(), LoggingStateError> {
    let mut kwargs = call.extra.clone();
    for (name, value) in values {
        kwargs.insert(name.clone(), param_json(value));
    }
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.litellm_call_id
            .as_ref()
            .map_or(Json::Null, |call_id| Json::Text(call_id.clone())),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        logging,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: None,
            optional_params: logged_optional_params(endpoint, values),
            additional_params,
        },
    )
}

// only the catalog's path params are logged; the query params never are
fn logged_optional_params(
    endpoint: &ContainerEndpoint,
    values: &BTreeMap<String, ParamValue>,
) -> IndexMap<String, Json> {
    let mut optional_params = IndexMap::new();
    for name in &endpoint.path_params {
        if let Some(value) = values.get(name) {
            optional_params.insert(name.clone(), param_json(value));
        }
    }
    optional_params
}

// `timeout or DEFAULT_REQUEST_TIMEOUT`: zero and none fall back, and the default is a float number of seconds
fn resolved_timeout(
    call: &ContainerFileCall,
    default_timeout_seconds: f64,
) -> Result<Option<Duration>, ContainerFileOpError> {
    if let Some(requested) = call.timeout.filter(|given| !given.is_zero()) {
        return Ok(Some(requested));
    }
    Duration::try_from_secs_f64(default_timeout_seconds)
        .map(Some)
        .map_err(|source| ContainerFileOpError::Timeout {
            seconds: default_timeout_seconds,
            source,
        })
}

// the mock short circuit: a string is parsed, a known response type decodes, and "raw" comes back as it parsed
fn mock_reply(
    endpoint: &ContainerEndpoint,
    mock: &Json,
) -> Result<ContainerFileReply, ContainerFileOpError> {
    let body = match mock {
        Json::Text(text) => serde_json::from_str::<Json>(text)
            .map_err(|source| ContainerFileOpError::MockJson { source })?,
        other => other.clone(),
    };
    match endpoint.response_type.as_str() {
        "ContainerFileListResponse" => decode_mock(endpoint, &body).map(ContainerFileReply::List),
        "ContainerFileObject" => decode_mock(endpoint, &body).map(ContainerFileReply::File),
        "DeleteContainerFileResponse" => {
            decode_mock(endpoint, &body).map(ContainerFileReply::Deleted)
        }
        _ => raw_reply(&body),
    }
}

fn decode_mock<T>(endpoint: &ContainerEndpoint, body: &Json) -> Result<T, ContainerFileOpError>
where
    T: DeserializeOwned,
{
    serde_json::to_value(body)
        .and_then(serde_json::from_value)
        .map_err(|source| ContainerFileOpError::MockModel {
            response_type: endpoint.response_type.clone(),
            source,
        })
}

fn raw_reply(body: &Json) -> Result<ContainerFileReply, ContainerFileOpError> {
    serde_json::to_value(body)
        .map(ContainerFileReply::Raw)
        .map_err(|source| ContainerFileOpError::MockJson { source })
}
