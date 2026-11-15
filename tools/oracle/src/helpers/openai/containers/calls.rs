use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig as _, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::containers::OpenAiContainerConfig;
use serde_json::{Map, Value, json};

use super::fixture::{call_config, failure};
use super::input::{
    ContentCall, FileListCall, HeadersCall, ListCall, MetadataCall, MetadataKind, ReplyCall,
    ResourceCall, ResourceKind, UrlCall,
};
use crate::helpers::HelperError;

pub(super) fn url(call: &UrlCall) -> Result<Value, Box<HelperError>> {
    let built = call_config(&call.globals, &call.environment)?;
    let litellm_params = Map::new();
    let resolved = built
        .get_complete_url(call.api_base.as_deref(), &litellm_params)
        .map_err(|error| failure(&error.to_string()))?;
    Ok(json!({"url": resolved}))
}

pub(super) fn list(
    built: &OpenAiContainerConfig,
    context: ContainerRequestContext<'_>,
    call: &ListCall,
) -> Value {
    let query = ContainerListQuery {
        after: call.after.as_deref(),
        limit: call.limit,
        order: call.order.as_deref(),
        extra_query: call.extra_query.as_ref(),
    };
    let (url, params) = built.transform_container_list_request(&call.api_base, context, &query);
    json!({"url": url, "params": params})
}

pub(super) fn file_list(
    built: &OpenAiContainerConfig,
    context: ContainerRequestContext<'_>,
    call: &FileListCall,
) -> Result<Value, Box<HelperError>> {
    let query = ContainerListQuery {
        after: call.after.as_deref(),
        limit: call.limit,
        order: call.order.as_deref(),
        extra_query: call.extra_query.as_ref(),
    };
    let (url, params) = built
        .transform_container_file_list_request(&call.container_id, &call.api_base, context, &query)
        .map_err(|error| failure(&error.to_string()))?;
    Ok(json!({"url": url, "params": params}))
}

pub(super) fn resource(
    built: &OpenAiContainerConfig,
    context: ContainerRequestContext<'_>,
    call: &ResourceCall,
) -> Result<Value, Box<HelperError>> {
    let (url, data) = match call.kind {
        ResourceKind::Retrieve => {
            built.transform_container_retrieve_request(&call.container_id, &call.api_base, context)
        }
        ResourceKind::Delete => {
            built.transform_container_delete_request(&call.container_id, &call.api_base, context)
        }
    }
    .map_err(|error| failure(&error.to_string()))?;
    Ok(json!({"url": url, "data": data}))
}

pub(super) fn content(
    built: &OpenAiContainerConfig,
    context: ContainerRequestContext<'_>,
    call: &ContentCall,
) -> Result<Value, Box<HelperError>> {
    let (url, params) = built
        .transform_container_file_content_request(
            &call.container_id,
            &call.file_id,
            &call.api_base,
            context,
        )
        .map_err(|error| failure(&error.to_string()))?;
    Ok(json!({"url": url, "params": params}))
}

pub(super) fn headers(call: &HeadersCall) -> Result<Value, Box<HelperError>> {
    let built = call_config(&call.globals, &call.environment)?;
    let resolved = built.validate_environment(&call.headers, call.api_key.as_deref());
    Ok(json!({"headers": resolved}))
}

pub(super) fn create_reply(built: &OpenAiContainerConfig, call: &ReplyCall) -> Value {
    match built.transform_container_create_response(call.body.as_bytes()) {
        Ok(record) => json!({
            "ok": true,
            "record": record,
            "hidden": record.hidden_params,
        }),
        Err(_error) => json!({"ok": false, "record": Value::Null, "hidden": Value::Null}),
    }
}

pub(super) fn metadata_reply(built: &OpenAiContainerConfig, call: &MetadataCall) -> Value {
    match decode_metadata(built, call) {
        Ok(record) => json!({"ok": true, "record": record}),
        Err(_error) => json!({"ok": false, "record": Value::Null}),
    }
}

fn decode_metadata(
    built: &OpenAiContainerConfig,
    call: &MetadataCall,
) -> Result<Value, ContainerError> {
    match call.kind {
        MetadataKind::Retrieve => built
            .transform_container_retrieve_response(call.body.as_bytes())
            .map(|record| json!(record)),
        MetadataKind::List => built
            .transform_container_list_response(call.body.as_bytes())
            .map(|record| json!(record)),
        MetadataKind::Delete => built
            .transform_container_delete_response(call.body.as_bytes())
            .map(|record| json!(record)),
        MetadataKind::FileList => built
            .transform_container_file_list_response(call.body.as_bytes())
            .map(|record| json!(record)),
    }
}

pub(super) fn content_reply(
    built: &OpenAiContainerConfig,
    call: &ReplyCall,
) -> Result<Value, Box<HelperError>> {
    let content = built
        .transform_container_file_content_response(call.body.as_bytes())
        .map_err(|error| failure(&error.to_string()))?;
    let text = String::from_utf8(content).map_err(|error| failure(&error.to_string()))?;
    Ok(json!({"content": text}))
}
