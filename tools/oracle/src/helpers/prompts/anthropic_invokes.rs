use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::anthropic_invokes::cache_control::add_cache_control_to_content;
use betterlite_prompts::anthropic_invokes::documents::{
    ContentBlockType, FileIdContentType, content_element_factory, infer_file_id_content_type,
    process_openai_file_message, select_content_block_type,
};
use betterlite_prompts::anthropic_invokes::invoke::to_anthropic_tool_invokes;
use betterlite_prompts::anthropic_results::images::ImageChunk;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolInvokeCall {
    tool_calls: Vec<Json>,
    #[serde(default)]
    web_search_results: Vec<Json>,
    #[serde(default)]
    tool_results: Vec<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheControlCall {
    element: Object,
    original: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentTypeCall {
    formats: Vec<String>,
    file_ids: Vec<String>,
    chunk: ImageChunk,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileMessageCall {
    messages: Vec<Object>,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "convert_to_anthropic_tool_invoke")]
    ToolInvoke(ToolInvokeCall),
    #[serde(rename = "add_cache_control_to_content")]
    CacheControl(CacheControlCall),
    #[serde(rename = "document_type")]
    DocumentType(DocumentTypeCall),
    #[serde(rename = "process_openai_file_message")]
    ProcessFileMessage(FileMessageCall),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToolInvoke(call) => serde_json::to_value(
                to_anthropic_tool_invokes(
                    &call.tool_calls,
                    &call.web_search_results,
                    &call.tool_results,
                )
                .map_err(|error| helper_error("InvokeError", None, error))?,
            )
            .map_err(|error| helper_error("SerializationError", None, error)),
            Input::CacheControl(call) => serde_json::to_value(add_cache_control_to_content(
                call.element.clone(),
                &call.original,
            ))
            .map_err(|error| helper_error("SerializationError", None, error)),
            Input::DocumentType(call) => document_type(call),
            Input::ProcessFileMessage(call) => process_file_messages(call),
        }
    }
}

struct MapSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl MapSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for MapSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.result_for(url)).collect())
    }
}

fn document_type(call: &DocumentTypeCall) -> HelperResult {
    let selected: Vec<ContentBlockType> = call
        .formats
        .iter()
        .map(|format| select_content_block_type(format))
        .collect();
    let inferred: Vec<FileIdContentType> = call
        .file_ids
        .iter()
        .map(|file_id| infer_file_id_content_type(file_id))
        .collect();
    serde_json::to_value(serde_json::json!({
        "select": selected,
        "infer": inferred,
        "factory": content_element_factory(&call.chunk),
    }))
    .map_err(|error| helper_error("SerializationError", None, error))
}

fn process_file_messages(call: &FileMessageCall) -> HelperResult {
    let source = MapSource {
        data_urls: &call.data_urls,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", None, error))?;
    let mut blocks = Vec::with_capacity(call.messages.len());
    for message in &call.messages {
        let block = runtime
            .block_on(process_openai_file_message(message, &source))
            .map_err(|error| helper_error("FileMessageError", Some(400), error))?;
        blocks.push(block);
    }
    serde_json::to_value(blocks).map_err(|error| helper_error("SerializationError", None, error))
}

fn helper_error<Message>(class: &str, status: Option<u16>, message: Message) -> Box<HelperError>
where
    Message: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
