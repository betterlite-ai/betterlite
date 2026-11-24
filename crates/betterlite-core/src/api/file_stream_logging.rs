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
use source::{SourceFailure, StreamError, StreamSource};
use stream::{AsyncIter, SyncIter};

pub struct FileStreamSetup {
    pub source: StreamSource,
    pub file_id: String,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub logger: Option<Arc<dyn FileStreamLogger>>,
}

// FileContentStreamingResponse: one logging flag and one close flag, so an early close never logs a success
pub struct FileContentStreamingResponse {
    source: StreamSource,
    file_id: String,
    model: Option<String>,
    custom_llm_provider: Option<String>,
    logger: Option<Arc<dyn FileStreamLogger>>,
    clock: Arc<dyn SuccessClock>,
    standard_logging_object: Option<StandardLoggingPayload>,
    hidden: HiddenParams,
    logging_completed: bool,
    close_completed: bool,
    start_time: DateTime<FixedOffset>,
}

impl FileContentStreamingResponse {
    #[must_use]
    pub fn new(setup: FileStreamSetup, clock: Arc<dyn SuccessClock>) -> Self {
        let logger_start = setup.logger.as_ref().and_then(|logger| logger.start_time());
        let start_time = match logger_start {
            Some(time) => time,
            None => clock.now(),
        };
        let mut stream = Self {
            source: setup.source,
            file_id: setup.file_id,
            model: setup.model,
            custom_llm_provider: setup.custom_llm_provider,
            logger: setup.logger,
            clock,
            standard_logging_object: None,
            hidden: HiddenParams::default(),
            logging_completed: false,
            close_completed: false,
            start_time,
        };
        stream.sync_hidden_params();
        stream
    }

    #[must_use]
    pub const fn logging_completed(&self) -> bool {
        self.logging_completed
    }

    #[must_use]
    pub const fn close_completed(&self) -> bool {
        self.close_completed
    }

    #[must_use]
    pub fn hidden_params(&self) -> &HiddenParams {
        &self.hidden
    }

    #[must_use]
    pub fn start_time(&self) -> DateTime<FixedOffset> {
        self.start_time
    }

    #[must_use]
    pub fn standard_logging_object(&self) -> Option<&StandardLoggingPayload> {
        self.standard_logging_object.as_ref()
    }

    pub fn sync_iter(&mut self) -> Result<SyncIter<'_>, StreamError> {
        if matches!(&self.source, StreamSource::Sync(_)) {
            return Ok(SyncIter::new(self));
        }
        Err(StreamError::UnsupportedSync)
    }

    pub fn async_iter(&mut self) -> Result<AsyncIter<'_>, StreamError> {
        if matches!(&self.source, StreamSource::Async(_)) {
            return Ok(AsyncIter::new(self));
        }
        Err(StreamError::UnsupportedAsync)
    }

    // _sync_hidden_params: api_base is written once, the model name is always corrected from the file id
    pub fn sync_hidden_params(&mut self) {
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
    pub fn standard_object(
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
            payload.model = model.clone();
        }
        if let Some(api_base) = hidden_api_base(&self.hidden) {
            payload.api_base = api_base.to_owned();
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

    fn failure_call(&self, exception: ErrorClass, end_time: DateTime<FixedOffset>) -> FailureCall {
        FailureCall {
            exception,
            traceback: String::new(),
            start_time: self.start_time,
            end_time,
        }
    }

    fn log_success_sync(&mut self) -> Result<(), StreamError> {
        if self.logging_completed {
            return Ok(());
        }
        let Some(logger) = self.logger.clone() else {
            return Ok(());
        };
        self.logging_completed = true;
        let end_time = self.clock.now();
        let payload = self.standard_object(end_time).cloned();
        let call = self.success_call(end_time, payload);
        logger
            .success_handler(call)
            .map_err(|failure| StreamError::Callback { failure })
    }

    async fn log_success_async(&mut self) -> Result<(), StreamError> {
        if self.logging_completed {
            return Ok(());
        }
        let Some(logger) = self.logger.clone() else {
            return Ok(());
        };
        self.logging_completed = true;
        let end_time = self.clock.now();
        let payload = self.standard_object(end_time).cloned();
        let call = self.success_call(end_time, payload);
        logger
            .async_success_handler(call)
            .await
            .map_err(|failure| StreamError::Callback { failure })?;
        let sync_call = self.success_call(end_time, None);
        logger
            .success_sync_callbacks(sync_call)
            .map_err(|failure| StreamError::Callback { failure })
    }

    fn log_failure_sync(&mut self, exception: ErrorClass) -> Result<(), StreamError> {
        if self.logging_completed {
            return Ok(());
        }
        let Some(logger) = self.logger.clone() else {
            return Ok(());
        };
        self.logging_completed = true;
        let end_time = self.clock.now();
        let call = self.failure_call(exception, end_time);
        logger
            .failure_handler(call)
            .map_err(|failure| StreamError::Callback { failure })
    }

    // LiteLLM calls the sync handler first and awaits the async one only when it did not raise
    async fn log_failure_async(&mut self, exception: ErrorClass) -> Result<(), StreamError> {
        if self.logging_completed {
            return Ok(());
        }
        let Some(logger) = self.logger.clone() else {
            return Ok(());
        };
        self.logging_completed = true;
        let end_time = self.clock.now();
        let call = self.failure_call(exception, end_time);
        logger
            .failure_handler(call.clone())
            .map_err(|failure| StreamError::Callback { failure })?;
        logger
            .async_failure_handler(call)
            .await
            .map_err(|failure| StreamError::Callback { failure })
    }

    fn fail_sync(&mut self, failure: SourceFailure) -> StreamError {
        match failure {
            SourceFailure::Exception(error) => {
                if let Err(callback) = self.log_failure_sync(error.clone()) {
                    return callback;
                }
                StreamError::Source { failure: error }
            }
            SourceFailure::Cancellation => StreamError::Cancelled,
        }
    }

    async fn fail_async(&mut self, failure: SourceFailure) -> StreamError {
        match failure {
            SourceFailure::Exception(error) => {
                if let Err(callback) = self.log_failure_async(error.clone()).await {
                    return callback;
                }
                StreamError::Source { failure: error }
            }
            SourceFailure::Cancellation => StreamError::Cancelled,
        }
    }
}
