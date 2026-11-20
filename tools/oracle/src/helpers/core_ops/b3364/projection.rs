use std::collections::BTreeMap;

use betterlite_core::api::container_collection_http::CollectionRequest;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::error::BaseLlmException;
use betterlite_openai::api::container_contracts::records::{
    ContainerListResponse, ContainerObject,
};
use betterlite_openai::containers::container_file_http::transport::ContainerMethod;
use serde_json::{Map, Value, json};
use url::Url;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

// What the transport saw for the one call, with the query rendered the way both sides put it on the wire
pub(super) fn request(requests: &[CollectionRequest]) -> Result<Value, Box<HelperError>> {
    let Some(sent) = requests.first() else {
        return Ok(json!({
            "method": null,
            "url": null,
            "headers": {},
            "body": null,
            "query": {},
            "timeout": null,
        }));
    };
    let mut parsed = Url::parse(&sent.url).or_value_error()?;
    let embedded = parsed
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()));
    let query: Map<String, Value> = embedded
        .chain(sent.query.iter().cloned())
        .map(|(name, value)| (name, Value::String(value)))
        .collect();
    parsed.set_query(None);
    let body = match sent.body.as_deref() {
        Some(bytes) => serde_json::from_slice(bytes).or_value_error()?,
        None => Value::Null,
    };
    let timeout = match sent.method {
        ContainerMethod::Post => sent.timeout.map(|held| Value::from(held.as_secs())),
        ContainerMethod::Get | ContainerMethod::Delete => None,
    };
    Ok(json!({
        "method": method_name(sent.method),
        "url": parsed.to_string(),
        "headers": wanted_headers(sent),
        "body": body,
        "query": query,
        "timeout": timeout,
    }))
}

const fn method_name(method: ContainerMethod) -> &'static str {
    match method {
        ContainerMethod::Post => "POST",
        ContainerMethod::Get => "GET",
        ContainerMethod::Delete => "DELETE",
    }
}

// A case compares the auth header and, for a create, the JSON content type; httpx's own defaults stay out
fn wanted_headers(sent: &CollectionRequest) -> Map<String, Value> {
    let mut names = vec![String::from("authorization")];
    if matches!(sent.method, ContainerMethod::Post) {
        names.push(String::from("content-type"));
    }
    sent.headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .filter(|(name, _)| names.contains(name))
        .map(|(name, value)| (name, Value::String(value.clone())))
        .collect()
}

pub(super) fn log(calls: &[(Json, AdditionalArgs)]) -> Result<Value, Box<HelperError>> {
    let entries = calls
        .iter()
        .map(call_record)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(Value::Array(entries))
}

fn call_record(call: &(Json, AdditionalArgs)) -> Result<Value, Box<HelperError>> {
    let (input, arguments) = call;
    let logged_input = serde_json::to_value(input).or_value_error()?;
    let headers = arguments
        .headers
        .as_ref()
        .map(|named| {
            let lowered: BTreeMap<String, &Json> = named
                .iter()
                .map(|(name, value)| (name.to_lowercase(), value))
                .collect();
            serde_json::to_value(lowered)
        })
        .transpose()
        .or_value_error()?;
    let body = match arguments.complete_input_dict.as_ref() {
        Some(RawBody::Object(fields)) => Some(serde_json::to_value(fields).or_value_error()?),
        Some(RawBody::Text(text)) => Some(Value::String(text.clone())),
        None => None,
    };
    let params = arguments
        .passthrough
        .get("params")
        .map(serde_json::to_value)
        .transpose()
        .or_value_error()?;
    Ok(json!({
        "input": logged_input,
        "api_base": arguments.api_base,
        "headers": headers,
        "body": body,
        "params": params,
    }))
}

pub(super) fn container(record: &ContainerObject) -> Value {
    json!({
        "id": &record.id,
        "object": "container",
        "created_at": record.created_at,
        "status": &record.status,
        "expires_after": &record.expires_after,
        "last_active_at": record.last_active_at,
        "name": &record.name,
    })
}

pub(super) fn page(record: &ContainerListResponse) -> Value {
    let data: Vec<Value> = record.data.iter().map(container).collect();
    json!({
        "object": "list",
        "data": data,
        "first_id": &record.first_id,
        "last_id": &record.last_id,
        "has_more": record.has_more,
    })
}

pub(super) fn provider(exception: &BaseLlmException) -> Value {
    json!({
        "status_code": exception.status_code,
        "message": exception.message,
    })
}
