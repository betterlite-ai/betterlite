use betterlite_prompts::anthropic_invokes::documents::{
    FileMessageError, process_openai_file_message,
};
use betterlite_prompts::anthropic_results::tool_result::{FileBlocks, to_anthropic_tool_result};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use futures_util::FutureExt as _;
use thiserror::Error;

use super::MessageError;

const FILE: &str = "file";
const FILE_DATA: &str = "file_data";
const HTTP_PREFIX: &str = "http";
const TOOL_CALL_ID: &str = "tool_call_id";
const CACHE_CONTROL: &str = "cache_control";

#[derive(Debug, Error)]
pub enum FileBlockError {
    #[error(transparent)]
    File(#[from] FileMessageError),
    #[error("a remote file URL inside a tool result could not be fetched")]
    Fetch(#[source] ImageFetchError),
    #[error("a remote file URL inside a tool result was not fetched before conversion")]
    Pending,
}

pub async fn tool_result_from_parts<Source>(
    parts: &[ContentPart],
    tool_call_id: &str,
    cache_control: Option<Json>,
    fetcher: &Source,
) -> Result<Object, MessageError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let parts = prefetch_file_data(parts, fetcher).await?;
    let mut fields =
        Object::from_iter([(TOOL_CALL_ID.to_owned(), Json::Text(tool_call_id.to_owned()))]);
    if let Some(value) = cache_control {
        fields.insert(CACHE_CONTROL.to_owned(), value);
    }
    let message = Message {
        role: "tool".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: fields,
    };
    let blocks = RemoteFileBlocks { fetcher };
    let id_source = String::new;
    Ok(to_anthropic_tool_result(&message, true, fetcher, &blocks, &id_source).await?)
}

// LiteLLM fetches a remote file_data synchronously; B223 is async, so the URL is fetched here first
async fn prefetch_file_data<Source>(
    parts: &[ContentPart],
    fetcher: &Source,
) -> Result<Vec<ContentPart>, FileBlockError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let mut prefetched = Vec::with_capacity(parts.len());
    for part in parts {
        prefetched.push(prefetch_part(part.clone(), fetcher).await?);
    }
    Ok(prefetched)
}

async fn prefetch_part<Source>(
    mut part: ContentPart,
    fetcher: &Source,
) -> Result<ContentPart, FileBlockError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let Some(Json::Object(file)) = part.fields.get_mut(FILE) else {
        return Ok(part);
    };
    let Some(Json::Text(data)) = file.get(FILE_DATA) else {
        return Ok(part);
    };
    if !data.starts_with(HTTP_PREFIX) {
        return Ok(part);
    }
    let fetched = fetcher
        .convert_url(data)
        .await
        .map_err(FileBlockError::Fetch)?;
    file.insert(FILE_DATA.to_owned(), Json::Text(fetched));
    Ok(part)
}

struct RemoteFileBlocks<'call, Source> {
    fetcher: &'call Source,
}

impl<Source> FileBlocks for RemoteFileBlocks<'_, Source>
where
    Source: RemoteMediaFetcher + Sync,
{
    type Failure = FileBlockError;

    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure> {
        let outcome = process_openai_file_message(element, self.fetcher).now_or_never();
        match outcome {
            Some(Ok(block)) => Ok(block),
            Some(Err(error)) => Err(FileBlockError::File(error)),
            None => Err(FileBlockError::Pending),
        }
    }
}
