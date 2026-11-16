use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig as _, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::containers::OpenAiContainerConfig;
use serde_json::{Map, Value, json};

use super::fixture::call_config;
use super::input::{BodyCall, HeadersCall, ReplyCall, ReplyKind, RequestCall, UrlCall};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn url(call: &UrlCall) -> HelperResult {
    let built = call_config(&call.globals, &call.environment)?;
    let resolved = built
        .get_complete_url(call.api_base.as_deref(), &Map::new())
        .or_value_error()?;
    Ok(json!({"url": resolved}))
}

// LiteLLM's encode_url_path_segment raises ValueError with the same text as the path-segment error
pub(super) fn request(
    built: &OpenAiContainerConfig,
    context: ContainerRequestContext<'_>,
    call: &RequestCall,
) -> HelperResult {
    let (url, params) = match call {
        RequestCall::List { api_base, query } => built.transform_container_list_request(
            api_base,
            context,
            &ContainerListQuery::from(query),
        ),
        RequestCall::FileList {
            container_id,
            api_base,
            query,
        } => built
            .transform_container_file_list_request(
                container_id,
                api_base,
                context,
                &ContainerListQuery::from(query),
            )
            .or_value_error()?,
        RequestCall::Retrieve {
            container_id,
            api_base,
        } => built
            .transform_container_retrieve_request(container_id, api_base, context)
            .or_value_error()?,
        RequestCall::Delete {
            container_id,
            api_base,
        } => built
            .transform_container_delete_request(container_id, api_base, context)
            .or_value_error()?,
        RequestCall::Content {
            container_id,
            file_id,
            api_base,
        } => built
            .transform_container_file_content_request(container_id, file_id, api_base, context)
            .or_value_error()?,
    };
    Ok(json!({"url": url, "params": params}))
}

pub(super) fn headers(call: &HeadersCall) -> HelperResult {
    let built = call_config(&call.globals, &call.environment)?;
    let resolved = built.validate_environment(&call.headers, call.api_key.as_deref());
    Ok(json!({"headers": resolved}))
}

pub(super) fn reply(built: &OpenAiContainerConfig, call: &ReplyCall) -> Value {
    let body = call.body.as_bytes();
    let decoded: Result<Value, ContainerError> = match call.kind {
        ReplyKind::Create => built
            .transform_container_create_response(body)
            .map(|record| json!({"ok": true, "record": record, "hidden": record.hidden_params})),
        ReplyKind::Retrieve => built
            .transform_container_retrieve_response(body)
            .map(|record| json!({"ok": true, "record": record})),
        ReplyKind::List => built
            .transform_container_list_response(body)
            .map(|record| json!({"ok": true, "record": record})),
        ReplyKind::Delete => built
            .transform_container_delete_response(body)
            .map(|record| json!({"ok": true, "record": record})),
        ReplyKind::FileList => built
            .transform_container_file_list_response(body)
            .map(|record| json!({"ok": true, "record": record})),
    };
    decoded.unwrap_or_else(|_error| json!({"ok": false}))
}

pub(super) fn content_reply(built: &OpenAiContainerConfig, call: &BodyCall) -> HelperResult {
    let content = built
        .transform_container_file_content_response(call.body.as_bytes())
        .or_value_error()?;
    Ok(json!({"content": String::from_utf8(content).or_value_error()?}))
}
