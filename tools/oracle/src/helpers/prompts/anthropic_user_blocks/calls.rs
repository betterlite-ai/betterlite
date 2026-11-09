use std::collections::BTreeMap;

use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::anthropic_user_blocks::prelude::prepare;
use betterlite_prompts::anthropic_user_blocks::system_prompt::get_system_prompt;
use betterlite_prompts::anthropic_user_blocks::user_run::UserBlockBuilder;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use serde::Deserialize;
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult};

use super::failures::{conversion_failure, failure, prepare_failure};
use super::policies::{NoFileBlocks, UrlSource};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserBlocksCall {
    messages: Vec<Json>,
    model: String,
    llm_provider: String,
    #[serde(default)]
    modify_params: bool,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemPromptCall {
    messages: Vec<Json>,
}

pub(crate) fn user_blocks(call: &UserBlocksCall) -> HelperResult {
    let prepared = prepare(
        &call.messages,
        &call.model,
        &call.llm_provider,
        call.modify_params,
    )
    .map_err(|error| prepare_failure(&error))?;
    let source = UrlSource {
        data_urls: &call.data_urls,
    };
    let file_blocks = NoFileBlocks;
    let new_id = String::new;
    let builder = UserBlockBuilder {
        force_base64: prepared.force_base64,
        fetcher: &source,
        blocks: &file_blocks,
        new_id: &new_id,
    };
    let messages = runtime()?.block_on(project(&builder, &prepared.messages))?;
    serde_json::to_value(messages).map_err(|error| failure(None, &error))
}

pub(crate) fn system_prompt(call: &SystemPromptCall) -> HelperResult {
    let (prompt, remaining) =
        get_system_prompt(&call.messages).map_err(|error| failure(None, &error))?;
    serde_json::to_value([Json::Text(prompt), Json::List(remaining)])
        .map_err(|error| failure(None, &error))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(None, &error))
}

async fn project<Fetcher, Blocks, Ids>(
    builder: &UserBlockBuilder<'_, Fetcher, Blocks, Ids>,
    messages: &[Json],
) -> Result<Vec<Json>, Box<HelperError>>
where
    Fetcher: RemoteMediaFetcher + Sync,
    Blocks: FileBlocks + Sync,
    Ids: Fn() -> String + Sync,
{
    let mut converted = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let run = builder
            .convert(messages, index)
            .await
            .map_err(|error| conversion_failure(&error))?;
        if run.next == index {
            return Err(failure(
                None,
                &format!("message at index {index} is not part of a user run"),
            ));
        }
        if let Some(message) = run.message() {
            converted.push(message);
        }
        index = run.next;
    }
    Ok(converted)
}
