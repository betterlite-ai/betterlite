use futures_util::FutureExt as _;
use thiserror::Error;

use crate::anthropic_invokes::documents::{FileMessageError, process_openai_file_message};
use crate::anthropic_results::tool_result::FileBlocks;
use crate::arguments::json::Object;
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;

#[derive(Debug, Error)]
pub enum FileBlockError {
    #[error(transparent)]
    File(#[from] FileMessageError),
    #[error("a remote file URL inside a tool result could not be fetched")]
    Fetch(#[source] ImageFetchError),
    #[error("a remote file URL inside a tool result was not fetched before conversion")]
    Pending,
}

pub struct RemoteFileBlocks<'call, Source> {
    pub fetcher: &'call Source,
}

impl<Source> FileBlocks for RemoteFileBlocks<'_, Source>
where
    Source: RemoteMediaFetcher + Sync,
{
    type Failure = FileBlockError;

    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure> {
        match process_openai_file_message(element, self.fetcher).now_or_never() {
            Some(Ok(block)) => Ok(block),
            Some(Err(error)) => Err(FileBlockError::File(error)),
            None => Err(FileBlockError::Pending),
        }
    }
}
