use std::mem;

use tokio::runtime::Handle;

use super::FileContentStreamingResponse;
use super::source::{
    AsyncByteSource, CloseResult, EmptySource, StreamError, StreamSource, SyncByteSource,
};

// The two close paths in one place: both flags are set before the source is taken, so a close never logs
impl FileContentStreamingResponse {
    pub fn close(&mut self) -> Result<(), StreamError> {
        if self.close_completed {
            return Ok(());
        }
        self.close_completed = true;
        self.logging_completed = true;
        let source = mem::replace(&mut self.source, StreamSource::Sync(Box::new(EmptySource)));
        match source {
            StreamSource::Sync(mut source) => sync_cleanup(source.close()),
            StreamSource::Async(mut source) => {
                let _pending = source.close();
                Ok(())
            }
        }
    }

    pub async fn aclose(&mut self) -> Result<(), StreamError> {
        if self.close_completed {
            return Ok(());
        }
        self.close_completed = true;
        self.logging_completed = true;
        let source = mem::replace(&mut self.source, StreamSource::Sync(Box::new(EmptySource)));
        match source {
            StreamSource::Sync(mut source) => sync_cleanup(source.close()),
            StreamSource::Async(source) => shielded_cleanup(source).await,
        }
    }
}

pub struct SyncIter<'stream> {
    stream: &'stream mut FileContentStreamingResponse,
}

pub struct AsyncIter<'stream> {
    stream: &'stream mut FileContentStreamingResponse,
}

impl<'stream> SyncIter<'stream> {
    pub(super) const fn new(stream: &'stream mut FileContentStreamingResponse) -> Self {
        Self { stream }
    }

    pub fn next_chunk(&mut self) -> Option<Result<Vec<u8>, StreamError>> {
        let item = {
            let StreamSource::Sync(source) = &mut self.stream.source else {
                return Some(Err(StreamError::UnsupportedSync));
            };
            source.next_chunk()
        };
        match item {
            Some(Ok(chunk)) => Some(Ok(chunk)),
            Some(Err(failure)) => Some(Err(self.stream.fail_sync(failure))),
            None => match self.stream.log_success_sync() {
                Ok(()) => None,
                Err(failure) => Some(Err(failure)),
            },
        }
    }
}

impl<'stream> AsyncIter<'stream> {
    pub(super) const fn new(stream: &'stream mut FileContentStreamingResponse) -> Self {
        Self { stream }
    }

    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, StreamError>> {
        let item = {
            let StreamSource::Async(source) = &mut self.stream.source else {
                return Some(Err(StreamError::UnsupportedAsync));
            };
            source.next_chunk().await
        };
        match item {
            Some(Ok(chunk)) => Some(Ok(chunk)),
            Some(Err(failure)) => Some(Err(self.stream.fail_async(failure).await)),
            None => match self.stream.log_success_async().await {
                Ok(()) => None,
                Err(failure) => Some(Err(failure)),
            },
        }
    }
}

fn sync_cleanup(outcome: Option<CloseResult>) -> Result<(), StreamError> {
    match outcome {
        Some(Err(failure)) => Err(StreamError::Cleanup { failure }),
        Some(Ok(())) | None => Ok(()),
    }
}

// close() on an async source: LiteLLM calls it and never awaits what it returns
fn drop_call(pending: CloseCall<'_>) {
    match pending {
        CloseCall::Awaitable(awaitable) => {
            drop(awaitable);
        }
        CloseCall::Absent | CloseCall::Returned => {}
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
        Err(error) => Err(StreamError::CleanupAborted {
            message: error.to_string(),
        }),
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
