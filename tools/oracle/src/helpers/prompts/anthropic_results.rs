mod calls;
mod failures;
mod policies;

use std::collections::BTreeMap;

use betterlite_prompts::anthropic_results::images::{ImageChunk, to_openai_image_url};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Alphabet {
    Anthropic,
    Bedrock,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ValueList {
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplaceIds {
    alphabet: Alphabet,
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolResultCall {
    message: Message,
    #[serde(default)]
    force_base64: bool,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FunctionInvokeCall {
    function_call: Object,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerResultCall {
    tool_id: String,
    #[serde(default)]
    web_search_results: Vec<Json>,
    #[serde(default)]
    tool_results: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkCall {
    chunk: ImageChunk,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageChunkCall {
    openai_image_url: String,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageParamCall {
    image_url_input: Json,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    is_bedrock_invoke: bool,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SanitizeAnthropicIds(ValueList),
    ReplaceInvalidIdChars(ReplaceIds),
    DocumentDataUris(ValueList),
    ToolResult(ToolResultCall),
    FunctionInvoke(FunctionInvokeCall),
    ServerToolResult(ServerResultCall),
    ImageChunkObject(ChunkCall),
    AnthropicImageChunk(ImageChunkCall),
    ImageParam(ImageParamCall),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SanitizeAnthropicIds(call) => calls::sanitize_ids(call),
            Input::ReplaceInvalidIdChars(call) => calls::replace_ids(call),
            Input::DocumentDataUris(call) => calls::document_uris(call),
            Input::ToolResult(call) => calls::tool_result(call),
            Input::FunctionInvoke(call) => calls::function_invoke(call),
            Input::ServerToolResult(call) => calls::server_tool_result(call),
            Input::ImageChunkObject(call) => calls::ok_value(to_openai_image_url(&call.chunk)),
            Input::AnthropicImageChunk(call) => calls::image_call(&calls::ImageCall::Chunk(call)),
            Input::ImageParam(call) => calls::image_call(&calls::ImageCall::Param(call)),
        }
    }
}
