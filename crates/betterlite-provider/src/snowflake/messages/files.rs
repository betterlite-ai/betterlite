use betterlite_prompts::anthropic_invokes::documents::{
    FileMessageError, process_openai_file_message,
};
use betterlite_prompts::anthropic_results::tool_result::{FileBlocks, to_anthropic_tool_result};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use futures_util::FutureExt as _;
use thiserror::Error;

use super::{Fetcher, MessageError};

#[derive(Debug, Error)]
pub enum FileBlockError {
    #[error(transparent)]
    File(#[from] FileMessageError),
    #[error("a remote file URL inside a tool result could not be fetched")]
    Fetch(#[source] ImageFetchError),
    #[error("a remote file URL inside a tool result was not fetched before conversion")]
    Pending,
}

pub async fn tool_result_from_parts<Source: Fetcher>(
    parts: &[ContentPart],
    tool_call_id: &str,
    cache_control: Option<Json>,
    fetcher: &Source,
) -> Result<Object, MessageError> {
    let parts = prefetch_file_data(parts, fetcher).await?;
    let mut fields = Object::from_iter([(
        "tool_call_id".to_owned(),
        Json::Text(tool_call_id.to_owned()),
    )]);
    if let Some(value) = cache_control {
        fields.insert("cache_control".to_owned(), value);
    }
    let message = Message {
        role: "tool".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: fields,
    };
    let blocks = RemoteFileBlocks { fetcher };
    let id_source = String::new();
    Ok(to_anthropic_tool_result(&message, true, fetcher, &blocks, &id_source).await?)
}

// LiteLLM fetches a remote file_data synchronously; B223 is async, so the URL is fetched here
async fn prefetch_file_data<Source: Fetcher>(
    parts: &[ContentPart],
    fetcher: &Source,
) -> Result<Vec<ContentPart>, FileBlockError> {
    let mut prefetched = Vec::with_capacity(parts.len());
    for part in parts {
        let mut copied = part.clone();
        if let Some(Json::Object(file)) = copied.fields.get_mut("file")
            && let Some(Json::Text(data)) = file.get("file_data")
            && data.starts_with("http")
        {
            let fetched = fetcher
                .convert_url(data)
                .await
                .map_err(FileBlockError::Fetch)?;
            file.insert("file_data".to_owned(), Json::Text(fetched));
        }
        prefetched.push(copied);
    }
    Ok(prefetched)
}

struct RemoteFileBlocks<'call, Source> {
    fetcher: &'call Source,
}

impl<Source: Fetcher> FileBlocks for RemoteFileBlocks<'_, Source> {
    type Failure = FileBlockError;

    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure> {
        match process_openai_file_message(element, self.fetcher).now_or_never() {
            Some(Ok(block)) => Ok(block),
            Some(Err(error)) => Err(FileBlockError::File(error)),
            None => Err(FileBlockError::Pending),
        }
    }
}
