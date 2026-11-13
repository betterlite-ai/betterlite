use std::future::Future;

use betterlite_prompts::anthropic_results::document_uri::is_anthropic_document_data_uri;
use betterlite_prompts::anthropic_results::function_invoke::to_anthropic_tool_use;
use betterlite_prompts::anthropic_results::ids::{
    replace_invalid_id_chars, sanitize_anthropic_tool_use_id,
};
use betterlite_prompts::anthropic_results::images::{
    create_anthropic_image_param, to_anthropic_image_chunk,
};
use betterlite_prompts::anthropic_results::server_result::find_server_tool_result;
use betterlite_prompts::anthropic_results::tool_result::to_anthropic_tool_result;
use serde::Serialize;
use tokio::runtime::Runtime;

use super::failures::{helper_failure, image_failure, tool_result_failure};
use super::policies::{CaseSource, NoBlocks, predicate};
use super::{
    FunctionInvokeCall, ImageChunkCall, ImageParamCall, ReplaceIds, ServerResultCall,
    ToolResultCall, ValueList,
};
use crate::helpers::{HelperError, HelperResult};

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_failure(None, error.to_string()))
}

pub(super) fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: Serialize,
{
    match serde_json::to_value(payload) {
        Ok(value) => Ok(value),
        Err(failure) => Err(helper_failure(None, failure.to_string())),
    }
}

fn block_on<Payload, Failure, Pending>(
    pending: Pending,
    failure: fn(&Failure) -> Box<HelperError>,
) -> HelperResult
where
    Pending: Future<Output = Result<Payload, Failure>>,
    Payload: Serialize,
{
    runtime()?
        .block_on(pending)
        .map_err(|error| failure(&error))
        .and_then(ok_value)
}

pub(super) fn sanitize_ids(call: &ValueList) -> HelperResult {
    let mut sanitized = Vec::with_capacity(call.values.len());
    for value in &call.values {
        sanitized.push(sanitize_anthropic_tool_use_id(value));
    }
    ok_value(sanitized)
}

pub(super) fn replace_ids(call: &ReplaceIds) -> HelperResult {
    let is_invalid = predicate(call.alphabet);
    let replaced = call
        .values
        .iter()
        .map(|value| replace_invalid_id_chars(value, is_invalid))
        .collect::<Vec<_>>();
    ok_value(replaced)
}

pub(super) fn document_uris(call: &ValueList) -> HelperResult {
    ok_value(
        call.values
            .iter()
            .map(|value| is_anthropic_document_data_uri(value))
            .collect::<Vec<_>>(),
    )
}

pub(super) fn tool_result(call: &ToolResultCall) -> HelperResult {
    let source = CaseSource {
        data_urls: &call.data_urls,
    };
    let blocks = NoBlocks;
    let new_id = || call.id.clone();
    block_on(
        to_anthropic_tool_result(&call.message, call.force_base64, &source, &blocks, &new_id),
        tool_result_failure,
    )
}

pub(super) fn function_invoke(call: &FunctionInvokeCall) -> HelperResult {
    let new_id = || call.id.clone();
    let calls = to_anthropic_tool_use(&call.function_call, &new_id)
        .map_err(|error| helper_failure(None, error.to_string()))?;
    ok_value(calls)
}

pub(super) fn server_tool_result(call: &ServerResultCall) -> HelperResult {
    ok_value(find_server_tool_result(
        &call.tool_id,
        &call.web_search_results,
        &call.tool_results,
    ))
}

pub(super) enum ImageCall<'call> {
    Chunk(&'call ImageChunkCall),
    Param(&'call ImageParamCall),
}

pub(super) fn image_call(call: &ImageCall<'_>) -> HelperResult {
    match call {
        ImageCall::Chunk(chunk_call) => {
            let source = CaseSource {
                data_urls: &chunk_call.data_urls,
            };
            block_on(
                to_anthropic_image_chunk(
                    &chunk_call.openai_image_url,
                    chunk_call.format.as_deref(),
                    &source,
                ),
                image_failure,
            )
        }
        ImageCall::Param(param_call) => {
            let source = CaseSource {
                data_urls: &param_call.data_urls,
            };
            block_on(
                create_anthropic_image_param(
                    &param_call.image_url_input,
                    param_call.format.as_deref(),
                    param_call.is_bedrock_invoke,
                    &source,
                ),
                image_failure,
            )
        }
    }
}
