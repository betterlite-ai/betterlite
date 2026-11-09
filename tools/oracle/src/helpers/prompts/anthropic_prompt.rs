use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt::Display;
use std::future::{Future, ready};

use betterlite_prompts::anthropic::assembly::{AssemblyError, PromptDeps, anthropic_messages_pt};
use betterlite_prompts::anthropic::error::AnthropicPromptError;
use betterlite_prompts::anthropic_invokes::documents::FileMessageError;
use betterlite_prompts::anthropic_results::images::ImageError;
use betterlite_prompts::anthropic_results::tool_result::{FileBlocks, ToolResultError};
use betterlite_prompts::anthropic_user_blocks::error::UserBlocksError;
use betterlite_prompts::anthropic_user_blocks::prelude::PrepareError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde::Deserialize;
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    messages: Vec<Json>,
    model: String,
    llm_provider: String,
    #[serde(default)]
    modify_params: bool,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
    #[serde(default = "default_id")]
    id: String,
}

fn default_id() -> String {
    "generated-id".to_owned()
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(None, &error))?;
        let fetcher = UrlSource {
            data_urls: &input.data_urls,
        };
        let blocks = NoFileBlocks;
        let new_id = || input.id.clone();
        let deps = PromptDeps {
            fetcher: &fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        let messages = runtime
            .block_on(anthropic_messages_pt(
                &input.messages,
                &input.model,
                &input.llm_provider,
                input.modify_params,
                &deps,
            ))
            .map_err(|error| failure(status_of(&error), &error))?;
        serde_json::to_value(messages).map_err(|error| failure(None, &error))
    }
}

struct UrlSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl RemoteMediaFetcher for UrlSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(
            self.data_urls
                .get(url)
                .cloned()
                .ok_or_else(|| ImageFetchError::Attempts {
                    url: url.to_owned(),
                }),
        )
    }

    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        let mut resolved = Vec::with_capacity(urls.len());
        for url in urls {
            resolved.push(self.convert_url(url).await?);
        }
        Ok(resolved)
    }
}

struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = FileBlocksRefused;

    fn file_block(&self, _element: &Object) -> Result<Object, FileBlocksRefused> {
        Err(FileBlocksRefused)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
struct FileBlocksRefused;

fn failure(status: Option<u16>, error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AnthropicPrompt".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// LiteLLM's no-message, unsupported-role, image and missing-file failures are BadRequestErrors; its other errors escape without a status
const fn status_of<FileFailure>(error: &AnthropicPromptError<FileFailure>) -> Option<u16>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    match error {
        AnthropicPromptError::Prepare(PrepareError::NoUserMessages)
        | AnthropicPromptError::Assembly(AssemblyError::UnsupportedRole { .. })
        | AnthropicPromptError::UserBlocks(
            UserBlocksError::Image(ImageError::Fetch(_))
            | UserBlocksError::ToolResult(ToolResultError::Image(ImageError::Fetch(_)))
            | UserBlocksError::File(
                FileMessageError::MissingFile | FileMessageError::Image(ImageError::Fetch(_)),
            ),
        ) => Some(400),
        AnthropicPromptError::Prepare(_)
        | AnthropicPromptError::Assistant(_)
        | AnthropicPromptError::Assembly(_)
        | AnthropicPromptError::UserBlocks(_) => None,
    }
}
