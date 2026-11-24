use std::mem;
use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

pub mod handler;
pub mod logging;
pub mod source;
pub mod stream;

mod tests;

use logging::{
    FailureCall, FileStreamLogger, HiddenField, HiddenParams, SuccessCall, hidden_api_base,
    logging_response, merged_hidden,
};
use source::{EmptySource, SourceFailure, StreamError, StreamSource};
use stream::{AsyncIter, SyncIter, aclose_source, close_source};

pub struct FileStreamSetup {
    pub source: StreamSource,
    pub file_id: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub logger: Option<Arc<dyn FileStreamLogger>>,
}

// the close flag lives on the wrapper and the logging flag on StreamLogging, so a close never logs
pub struct FileContentStreamingResponse {
    source: StreamSource,
    logging: StreamLogging,
    close_completed: bool,
}

// The logging side of the wrapper: the payload cache, the hidden params and the one completion flag
struct StreamLogging {
    file_id: String,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    logger: Option<Arc<dyn FileStreamLogger>>,
    clock: Arc<dyn SuccessClock>,
    standard_logging_object: Option<StandardLoggingPayload>,
    hidden: HiddenParams,
    logging_completed: bool,
    start_time: DateTime<FixedOffset>,
}

impl FileContentStreamingResponse {
    #[must_use]
    pub fn new(setup: FileStreamSetup, clock: Arc<dyn SuccessClock>) -> Self {
        let logger_start = setup.logger.as_ref().and_then(|logger| logger.start_time());
        let start_time = logger_start.unwrap_or_else(|| clock.now());
        let mut logging = StreamLogging {
            file_id: setup.file_id,
            model: setup.model,
            custom_llm_provider: setup.custom_llm_provider,
            logger: setup.logger,
            clock,
            standard_logging_object: None,
            hidden: HiddenParams::default(),
            logging_completed: false,
            start_time,
        };
        logging.sync_hidden_params();
        Self {
            source: setup.source,
            logging,
            close_completed: false,
        }
    }

    pub fn sync_iter(&mut self) -> Result<SyncIter<'_>, StreamError> {
        let source = self.source.sync_source()?;
        Ok(SyncIter {
            source,
            logging: &mut self.logging,
        })
    }

    pub fn async_iter(&mut self) -> Result<AsyncIter<'_>, StreamError> {
        let source = self.source.async_source()?;
        Ok(AsyncIter {
            source,
            logging: &mut self.logging,
        })
    }

    pub fn close(&mut self) -> Result<(), StreamError> {
        let Some(source) = self.take_source() else {
            return Ok(());
        };
        close_source(source)
    }

    pub async fn aclose(&mut self) -> Result<(), StreamError> {
        let Some(source) = self.take_source() else {
            return Ok(());
        };
        aclose_source(source).await
    }

    // The two flags and the empty-source swap happen together, so a close never logs and never cleans up twice
    fn take_source(&mut self) -> Option<StreamSource> {
        if self.close_completed {
            return None;
        }
        self.close_completed = true;
        self.logging.logging_completed = true;
        Some(mem::replace(
            &mut self.source,
            StreamSource::Sync(Box::new(EmptySource)),
        ))
    }
}

impl StreamLogging {
    fn begin_logging(&mut self) -> Option<(Arc<dyn FileStreamLogger>, DateTime<FixedOffset>)> {
        if self.logging_completed {
            return None;
        }
        let logger = self.logger.clone()?;
        self.logging_completed = true;
        Some((logger, self.clock.now()))
    }

    fn success_logging_start(&mut self) -> Option<(Arc<dyn FileStreamLogger>, SuccessCall)> {
        let (logger, end_time) = self.begin_logging()?;
        let payload = self.standard_object(end_time).cloned();
        Some((logger, self.success_call(end_time, payload)))
    }

    fn failure_logging_start(
        &mut self,
        exception: ErrorClass,
    ) -> Option<(Arc<dyn FileStreamLogger>, FailureCall)> {
        let (logger, end_time) = self.begin_logging()?;
        Some((logger, self.failure_call(exception, end_time)))
    }

    // _sync_hidden_params: api_base is written once, the model name is always corrected from the file id
    fn sync_hidden_params(&mut self) {
        let details = self
            .logger
            .as_ref()
            .and_then(|logger| logger.details_api_base());
        if matches!(&self.hidden.api_base, HiddenField::Absent)
            && let Some(value) = details.filter(|value| !value.is_empty())
        {
            self.hidden.api_base = HiddenField::Value(value);
        }
        self.hidden.litellm_model_name = self.model.clone();
        if matches!(&self.hidden.response_cost, HiddenField::Absent) {
            self.hidden.response_cost = HiddenField::Null;
        }
    }

    // _build_standard_logging_object: a built payload is cached unchanged
    fn standard_object(
        &mut self,
        end_time: DateTime<FixedOffset>,
    ) -> Option<&StandardLoggingPayload> {
        if self.standard_logging_object.is_none() {
            let payload = self.compose_payload(end_time);
            self.standard_logging_object = payload;
        }
        self.standard_logging_object.as_ref()
    }

    fn compose_payload(
        &mut self,
        end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let logger = self.logger.clone()?;
        self.sync_hidden_params();
        let response = logging_response(&self.file_id, self.model.as_deref());
        let mut payload =
            logger.build_payload(&Json::Object(response.clone()), self.start_time, end_time)?;
        payload.hidden_params = merged_hidden(payload.hidden_params, &self.hidden);
        payload.response = Some(Json::Object(response));
        if let Some(provider) = &self.custom_llm_provider {
            payload.custom_llm_provider = Some(provider.clone());
        }
        if let Some(model) = &self.model {
            payload.model.clone_from(model);
        }
        if let Some(api_base) = hidden_api_base(&self.hidden) {
            api_base.clone_into(&mut payload.api_base);
        }
        Some(payload)
    }

    fn success_call(
        &self,
        end_time: DateTime<FixedOffset>,
        standard_logging_object: Option<StandardLoggingPayload>,
    ) -> SuccessCall {
        SuccessCall {
            result: Json::Object(logging_response(&self.file_id, self.model.as_deref())),
            start_time: self.start_time,
            end_time,
            standard_logging_object,
        }
    }

    const fn failure_call(
        &self,
        exception: ErrorClass,
        end_time: DateTime<FixedOffset>,
    ) -> FailureCall {
        FailureCall {
            exception,
            traceback: String::new(),
            start_time: self.start_time,
            end_time,
        }
    }

    fn log_success_sync(&mut self) -> Result<(), StreamError> {
        let Some((logger, call)) = self.success_logging_start() else {
            return Ok(());
        };
        logger.success_handler(call).map_err(StreamError::from)
    }

    // LiteLLM awaits async_success_handler, then runs the sync callbacks with no payload
    async fn log_success_async(&mut self) -> Result<(), StreamError> {
        let Some((logger, call)) = self.success_logging_start() else {
            return Ok(());
        };
        let end_time = call.end_time;
        logger
            .async_success_handler(call)
            .await
            .map_err(StreamError::from)?;
        logger
            .success_sync_callbacks(self.success_call(end_time, None))
            .map_err(StreamError::from)
    }

    fn log_failure_sync(&mut self, exception: ErrorClass) -> Result<(), StreamError> {
        let Some((logger, call)) = self.failure_logging_start(exception) else {
            return Ok(());
        };
        logger.failure_handler(call).map_err(StreamError::from)
    }

    // LiteLLM calls the sync failure handler first and awaits the async one only when it did not raise
    async fn log_failure_async(&mut self, exception: ErrorClass) -> Result<(), StreamError> {
        let Some((logger, call)) = self.failure_logging_start(exception) else {
            return Ok(());
        };
        logger
            .failure_handler(call.clone())
            .map_err(StreamError::from)?;
        logger
            .async_failure_handler(call)
            .await
            .map_err(StreamError::from)
    }

    fn fail_sync(&mut self, failure: SourceFailure) -> StreamError {
        let SourceFailure::Exception(error) = failure else {
            return StreamError::Cancelled;
        };
        let logged = self.log_failure_sync(error.clone());
        StreamError::from_failure(error, logged)
    }

    async fn fail_async(&mut self, failure: SourceFailure) -> StreamError {
        let SourceFailure::Exception(error) = failure else {
            return StreamError::Cancelled;
        };
        let logged = self.log_failure_async(error.clone()).await;
        StreamError::from_failure(error, logged)
    }
}
