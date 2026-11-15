use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_core::api::messages_stream_logging::seams::ReportedFailure;
use betterlite_core::api::messages_stream_logging::slots::DetachedFailureHook;

use super::call::Close;
use super::recording::value_error;

pub(super) struct Upstream {
    pub(super) close: Close,
}

#[async_trait]
impl MessagesProviderStream for Upstream {
    type Error = ReportedFailure;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ReportedFailure> {
        Ok(None)
    }

    async fn aclose(&mut self) -> Option<Result<(), ReportedFailure>> {
        match self.close {
            Close::Unsupported => None,
            Close::Closes => Some(Ok(())),
            Close::Fails => Some(Err(value_error("closed"))),
        }
    }
}

pub(super) struct RecordingHook {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) raises: bool,
}

#[async_trait]
impl DetachedFailureHook for RecordingHook {
    async fn on_detached_failure(
        &self,
        exception: &ReportedFailure,
    ) -> Result<(), ReportedFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exception.to_string());
        if self.raises {
            return Err(value_error("hook"));
        }
        Ok(())
    }
}
