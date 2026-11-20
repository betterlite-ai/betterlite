use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::{
    ContainerCreateRequestParams, ContainerListOptionalParams,
};
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::json;

use super::{ContainerCreateCall, ContainerListCall};

// the create named arguments as LiteLLM's local_vars holds them, before the whitelist runs
pub(crate) fn create_request(call: &ContainerCreateCall) -> ContainerCreateRequestParams {
    ContainerCreateRequestParams {
        expires_after: call.expires_after.clone(),
        file_ids: call.file_ids.clone(),
        extra_headers: call.extra_headers.clone(),
        extra_body: call.extra_body.clone(),
        name: Some(call.name.clone()),
    }
}

// the list named arguments as LiteLLM's local_vars holds them, before the whitelist runs
pub(crate) fn list_request(call: &ContainerListCall) -> ContainerListOptionalParams {
    ContainerListOptionalParams {
        after: call.after.clone(),
        limit: call.limit,
        order: call.order.clone(),
        extra_headers: call.extra_headers.clone(),
        extra_query: rendered_query(call.extra_query.as_ref()),
    }
}

// B3360's list record carries text values, so a query value is the text it sends
fn rendered_query(
    query: Option<&IndexMap<String, ParamValue>>,
) -> Option<BTreeMap<String, String>> {
    let Some(values) = query else {
        return None;
    };
    Some(
        values
            .iter()
            .map(|(name, value)| (name.clone(), value.render()))
            .collect(),
    )
}

// B3364's list handler takes the caller's query as JSON values, text and integers as given
pub(crate) fn query_object(query: Option<&IndexMap<String, ParamValue>>) -> Option<Object> {
    let Some(values) = query else {
        return None;
    };
    let mut object = Object::new();
    for (name, value) in values {
        let rendered = match value {
            ParamValue::Text(text) => json!(text),
            ParamValue::Integer(number) => json!(number),
        };
        object.insert(name.clone(), rendered);
    }
    Some(object)
}

// B3361's encode_container_id_in_response reads extra_body as JSON values
pub(crate) fn extra_body_json(
    extra_body: Option<&BTreeMap<String, String>>,
) -> Option<IndexMap<String, JsonValue>> {
    let Some(values) = extra_body else {
        return None;
    };
    Some(
        values
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
            .collect(),
    )
}

// GenericLiteLLMParams(api_key=api_key, api_base=api_base, api_version=api_version, **kwargs)
pub(crate) fn request_params(
    kwargs: &IndexMap<String, Json>,
    api_key: Option<&str>,
    api_base: Option<&str>,
    api_version: Option<&str>,
) -> Result<Object, serde_json::Error> {
    let encoded = serde_json::to_string(kwargs)?;
    let mut params: Object = serde_json::from_str(&encoded)?;
    let credentials = [
        ("api_key", api_key),
        ("api_base", api_base),
        ("api_version", api_version),
    ];
    for (name, candidate) in credentials {
        if let Some(value) = candidate {
            params.insert(name.to_owned(), json!(value));
        }
    }
    Ok(params)
}

// LiteLLM pops the logging object and the async flag before GenericLiteLLMParams and the logging update see the kwargs
pub(crate) fn call_kwargs(
    extra: &IndexMap<String, Json>,
    litellm_metadata: Option<&IndexMap<String, JsonValue>>,
    litellm_call_id: Option<&str>,
) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut kwargs = extra.clone();
    kwargs.shift_remove("async_call");
    kwargs.shift_remove("litellm_logging_obj");
    if let Some(call_id) = litellm_call_id {
        kwargs.insert("litellm_call_id".to_owned(), Json::Text(call_id.to_owned()));
    }
    if let Some(metadata) = litellm_metadata {
        let encoded = serde_json::to_string(metadata)?;
        let value: Json = serde_json::from_str(&encoded)?;
        kwargs.insert("litellm_metadata".to_owned(), value);
    }
    Ok(kwargs)
}

// dict(container_create_request_params): what update_from_kwargs logs as optional_params
pub(crate) fn logged_params<T>(value: &T) -> Result<IndexMap<String, Json>, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    serde_json::from_value(serde_json::to_value(value)?)
}

// the same params as the Object B3364's handler reads
pub(crate) fn handler_params<T>(value: &T) -> Result<Object, serde_json::Error>
where
    T: Serialize + ?Sized,
{
    serde_json::from_value(serde_json::to_value(value)?)
}
