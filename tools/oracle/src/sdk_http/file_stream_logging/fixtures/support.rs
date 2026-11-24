use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_core::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use betterlite_core::api::file_stream_logging::source::{
    AsyncByteSource, CloseCall, CloseResult, SourceFailure,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::decisions::Json;
use betterlite_openai::files::stream::FileContentStream;
use chrono::{DateTime, FixedOffset, Utc};
use futures_util::StreamExt as _;

pub(crate) struct FaultyStream {
    pub(crate) stream: FileContentStream,
    pub(crate) served: usize,
    pub(crate) fail_after: Option<usize>,
}

#[async_trait]
impl AsyncByteSource for FaultyStream {
    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, SourceFailure>> {
        if Some(self.served) == self.fail_after {
            return Some(Err(SourceFailure::Exception(context(String::from(
                "broken",
            )))));
        }
        match self.stream.next().await {
            Some(Ok(bytes)) => {
                self.served = self.served.saturating_add(1);
                Some(Ok(bytes))
            }
            Some(Err(error)) => Some(Err(SourceFailure::Exception(context(error.to_string())))),
            None => None,
        }
    }

    async fn aclose(&mut self) -> Option<CloseResult> {
        None
    }

    fn close(&mut self) -> CloseCall<'_> {
        CloseCall::Absent
    }
}

fn context(message: String) -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message,
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

#[derive(Default)]
pub(crate) struct FakeLogger {
    pub(crate) success: AtomicUsize,
    pub(crate) failure: AtomicUsize,
}

#[async_trait]
impl FileStreamLogger for FakeLogger {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        None
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        None
    }

    fn success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        self.success.fetch_add(1_usize, Ordering::SeqCst);
        Ok(())
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        self.failure.fetch_add(1_usize, Ordering::SeqCst);
        Ok(())
    }
}

pub(crate) struct WallClock;

impl SuccessClock for WallClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}
