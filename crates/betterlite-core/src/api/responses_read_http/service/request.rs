use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use http::HeaderMap as HttpHeaders;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::super::config::{ReadConfig, ReadOperation, ReadResource};
use super::super::error::ReadError;
use super::{ClientKind, ClientSource, Operation, ReadCall, ReadDeps, Selected};

pub(super) fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow ReadDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
    operation: Operation,
) -> Result<Selected<'borrow, S::Client>, ReadError>
where
    'deps: 'borrow,
    C: ReadConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.kind == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    if mode == ClientKind::Asynchronous {
        // LiteLLM logs the shared session the async client is built beside; betterlite's pool shares none
        let label = operation.client_label();
        tracing::debug!("Creating HTTP client for {label} with shared_session: None");
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

pub(super) fn request_endpoint<C>(
    config: &C,
    call: &ReadCall<'_>,
) -> Result<(String, ReadResource), ReadError>
where
    C: ReadConfig,
{
    let target = call.target();
    let api_base = config.complete_url(target.params);
    let operation = match call {
        ReadCall::Retrieve(_) => ReadOperation::Retrieve,
        ReadCall::InputItems(items) => ReadOperation::InputItems(items.query),
    };
    let resource = config.read_resource(target.response_id, &api_base, target.params, operation)?;
    Ok((api_base, resource))
}

pub(super) fn query_pairs(
    query: &IndexMap<String, Json>,
) -> Result<Vec<(String, String)>, ReadError> {
    let mut pairs = Vec::with_capacity(query.len());
    for (name, value) in query {
        let text = match value {
            Json::Text(text) => text.clone(),
            Json::Number(number) => number.to_string(),
            Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
                return Err(ReadError::QueryValue { name: name.clone() });
            }
        };
        pairs.push((name.clone(), text));
    }
    Ok(pairs)
}

pub(super) fn wire_headers(headers: &BTreeMap<String, String>) -> Result<HttpHeaders, ReadError> {
    let mut map = HttpHeaders::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| ReadError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| ReadError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}
