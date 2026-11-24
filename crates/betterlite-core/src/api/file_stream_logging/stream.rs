use tokio::runtime::Handle;

use super::StreamLogging;
use super::source::{
    AsyncByteSource, CloseCall, CloseResult, StreamError, StreamSource, SyncByteSource,
};

// The wrapper set both completion flags and swapped in the empty source before either close runs
pub(super) fn close_source(source: StreamSource) -> Result<(), StreamError> {
    match source {
        StreamSource::Sync(mut inner) => sync_cleanup(inner.close()),
        StreamSource::Async(mut inner) => {
            let _pending = inner.close();
            Ok(())
        }
    }
}

pub(super) async fn aclose_source(source: StreamSource) -> Result<(), StreamError> {
    match source {
        StreamSource::Sync(mut inner) => sync_cleanup(inner.close()),
        StreamSource::Async(inner) => shielded_cleanup(inner).await,
    }
}

pub struct SyncIter<'stream> {
    pub(super) source: &'stream mut dyn SyncByteSource,
    pub(super) logging: &'stream mut StreamLogging,
}

pub struct AsyncIter<'stream> {
    pub(super) source: &'stream mut dyn AsyncByteSource,
    pub(super) logging: &'stream mut StreamLogging,
}

impl SyncIter<'_> {
    pub fn next_chunk(&mut self) -> Option<Result<Vec<u8>, StreamError>> {
        match self.source.next_chunk() {
            Some(Ok(chunk)) => Some(Ok(chunk)),
            Some(Err(failure)) => Some(Err(self.logging.fail_sync(failure))),
            None => self.logging.log_success_sync().err().map(Err),
        }
    }
}

impl AsyncIter<'_> {
    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, StreamError>> {
        match self.source.next_chunk().await {
            Some(Ok(chunk)) => Some(Ok(chunk)),
            Some(Err(failure)) => Some(Err(self.logging.fail_async(failure).await)),
            None => self.logging.log_success_async().await.err().map(Err),
        }
    }
}

fn sync_cleanup(outcome: Option<CloseResult>) -> Result<(), StreamError> {
    match outcome {
        Some(Err(failure)) => Err(StreamError::Cleanup { failure }),
        Some(Ok(())) | None => Ok(()),
    }
}

// anyio.CancelScope(shield=True): the cleanup runs on its own task, so dropping the request future cannot stop it
async fn shielded_cleanup(source: Box<dyn AsyncByteSource>) -> Result<(), StreamError> {
    let Some(handle) = Handle::try_current().ok() else {
        return async_cleanup(source)
            .await
            .map_err(|failure| StreamError::Cleanup { failure });
    };
    let task = handle.spawn(async move { async_cleanup(source).await });
    match task.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(failure)) => Err(StreamError::Cleanup { failure }),
        Err(error) => Err(StreamError::CleanupAborted { source: error }),
    }
}

// aclose preferred, otherwise close with a non-null result awaited
async fn async_cleanup(mut source: Box<dyn AsyncByteSource>) -> CloseResult {
    match source.aclose().await {
        Some(result) => result,
        None => match source.close() {
            CloseCall::Awaitable(awaitable) => awaitable.await,
            CloseCall::Absent | CloseCall::Returned => Ok(()),
        },
    }
}
