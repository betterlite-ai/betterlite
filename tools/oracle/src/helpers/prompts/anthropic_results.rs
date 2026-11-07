use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::anthropic_results::document_uri::is_anthropic_document_data_uri;
use betterlite_prompts::anthropic_results::function_invoke::{InvokeError, to_anthropic_tool_use};
use betterlite_prompts::anthropic_results::ids::{
    replace_invalid_id_chars, sanitize_anthropic_tool_use_id,
};
use betterlite_prompts::anthropic_results::images::{
    ImageChunk, ImageError, create_anthropic_image_param, to_anthropic_image_chunk,
    to_openai_image_url,
};
use betterlite_prompts::anthropic_results::server_result::find_server_tool_result;
use betterlite_prompts::anthropic_results::tool_result::{
    FileBlocks, ToolResultError, to_anthropic_tool_result,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde::Serialize;
use tokio::runtime::Runtime;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Alphabet {
    Anthropic,
    Bedrock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SanitizeAnthropicIds {
        values: Vec<String>,
    },
    ReplaceInvalidIdChars {
        alphabet: Alphabet,
        values: Vec<String>,
    },
    DocumentDataUris {
        values: Vec<String>,
    },
    ToolResult {
        message: Message,
        #[serde(default)]
        force_base64: bool,
        #[serde(default)]
        data_urls: BTreeMap<String, String>,
        id: String,
    },
    FunctionInvoke {
        function_call: Object,
        id: String,
    },
    ServerToolResult {
        tool_id: String,
        #[serde(default)]
        web_search_results: Option<Vec<Json>>,
        #[serde(default)]
        tool_results: Option<Vec<Json>>,
    },
    ImageChunkObject {
        chunk: ImageChunk,
    },
    AnthropicImageChunk {
        openai_image_url: String,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        data_urls: BTreeMap<String, String>,
    },
    ImageParam {
        image_url_input: Json,
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        is_bedrock_invoke: bool,
        #[serde(default)]
        data_urls: BTreeMap<String, String>,
    },
}

struct CaseSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl CaseSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for CaseSource<'_> {
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

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file content blocks")]
struct NoFileBlock;

struct NoBlocks;

impl FileBlocks for NoBlocks {
    type Failure = NoFileBlock;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileBlock> {
        Err(NoFileBlock)
    }
}

fn predicate(alphabet: Alphabet) -> fn(char) -> bool {
    match alphabet {
        Alphabet::Anthropic => {
            |candidate| !(candidate.is_ascii_alphanumeric() || candidate == '_' || candidate == '-')
        }
        Alphabet::Bedrock => |candidate| {
            !(candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '.' | ':' | '-'))
        },
    }
}

fn helper_failure(status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicResults".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn image_failure(error: &ImageError) -> Box<HelperError> {
    let status = match error {
        ImageError::Fetch(_) => Some(400),
        ImageError::DataUrl | ImageError::Input | ImageError::Field { .. } => None,
    };
    helper_failure(status, error.to_string())
}

fn tool_result_failure<Failure>(error: &ToolResultError<Failure>) -> Box<HelperError>
where
    Failure: std::error::Error + Send + Sync + 'static,
{
    let status = match error {
        ToolResultError::Image(ImageError::Fetch(_)) => Some(400),
        ToolResultError::Image(
            ImageError::DataUrl | ImageError::Input | ImageError::Field { .. },
        )
        | ToolResultError::Role { .. }
        | ToolResultError::MissingToolCallId
        | ToolResultError::Field { .. }
        | ToolResultError::TooManyParts { .. }
        | ToolResultError::File(_) => None,
    };
    helper_failure(status, error.to_string())
}

fn invoke_failure(error: &InvokeError) -> Box<HelperError> {
    let status = match error {
        InvokeError::Arguments(_) | InvokeError::Field { .. } => None,
    };
    helper_failure(status, error.to_string())
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_failure(None, error.to_string()))
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: Serialize,
{
    match serde_json::to_value(payload) {
        Ok(value) => Ok(value),
        Err(failure) => Err(helper_failure(None, failure.to_string())),
    }
}

fn sanitize_ids(values: &[String]) -> HelperResult {
    let mut sanitized = Vec::with_capacity(values.len());
    for value in values {
        sanitized.push(sanitize_anthropic_tool_use_id(value));
    }
    ok_value(sanitized)
}

fn replace_ids(alphabet: Alphabet, values: &[String]) -> HelperResult {
    let is_invalid = predicate(alphabet);
    ok_value(
        values
            .iter()
            .map(|value| replace_invalid_id_chars(value, is_invalid))
            .collect::<Vec<_>>(),
    )
}

fn document_uris(values: &[String]) -> HelperResult {
    ok_value(
        values
            .iter()
            .map(|value| is_anthropic_document_data_uri(value))
            .collect::<Vec<_>>(),
    )
}

fn tool_result(
    message: &Message,
    force_base64: bool,
    data_urls: &BTreeMap<String, String>,
    id: &str,
) -> HelperResult {
    let source = CaseSource { data_urls };
    let blocks = NoBlocks;
    let new_id = || id.to_owned();
    let converted = runtime()?
        .block_on(to_anthropic_tool_result(
            message,
            force_base64,
            &source,
            &blocks,
            &new_id,
        ))
        .map_err(|error| tool_result_failure(&error))?;
    ok_value(converted)
}

fn function_invoke(function_call: &Object, id: &str) -> HelperResult {
    let new_id = || id.to_owned();
    let calls =
        to_anthropic_tool_use(function_call, &new_id).map_err(|error| invoke_failure(&error))?;
    ok_value(calls)
}

fn server_tool_result(
    tool_id: &str,
    web_search_results: Option<&[Json]>,
    tool_results: Option<&[Json]>,
) -> HelperResult {
    ok_value(find_server_tool_result(
        tool_id,
        web_search_results,
        tool_results,
    ))
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

fn anthropic_image_chunk(
    openai_image_url: &str,
    format: Option<&str>,
    data_urls: &BTreeMap<String, String>,
) -> HelperResult {
    let source = CaseSource { data_urls };
    block_on(
        to_anthropic_image_chunk(openai_image_url, format, &source),
        image_failure,
    )
}

fn image_param(
    image_url_input: &Json,
    format: Option<&str>,
    is_bedrock_invoke: bool,
    data_urls: &BTreeMap<String, String>,
) -> HelperResult {
    let source = CaseSource { data_urls };
    block_on(
        create_anthropic_image_param(image_url_input, format, is_bedrock_invoke, &source),
        image_failure,
    )
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SanitizeAnthropicIds { values } => sanitize_ids(values),
            Input::ReplaceInvalidIdChars { alphabet, values } => replace_ids(*alphabet, values),
            Input::DocumentDataUris { values } => document_uris(values),
            Input::ToolResult {
                message,
                force_base64,
                data_urls,
                id,
            } => tool_result(message, *force_base64, data_urls, id),
            Input::FunctionInvoke { function_call, id } => function_invoke(function_call, id),
            Input::ServerToolResult {
                tool_id,
                web_search_results,
                tool_results,
            } => server_tool_result(
                tool_id,
                web_search_results.as_deref(),
                tool_results.as_deref(),
            ),
            Input::ImageChunkObject { chunk } => ok_value(to_openai_image_url(chunk)),
            Input::AnthropicImageChunk {
                openai_image_url,
                format,
                data_urls,
            } => anthropic_image_chunk(openai_image_url, format.as_deref(), data_urls),
            Input::ImageParam {
                image_url_input,
                format,
                is_bedrock_invoke,
                data_urls,
            } => image_param(
                image_url_input,
                format.as_deref(),
                *is_bedrock_invoke,
                data_urls,
            ),
        }
    }
}
