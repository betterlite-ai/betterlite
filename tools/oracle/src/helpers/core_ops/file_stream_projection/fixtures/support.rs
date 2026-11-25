use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::json;

pub(crate) struct CaptureLogger {
    pub(crate) api_base: Option<String>,
    pub(crate) payload: Option<StandardLoggingPayload>,
    pub(crate) recorded: Mutex<Option<StandardLoggingPayload>>,
}

#[async_trait]
impl FileStreamLogger for CaptureLogger {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        self.api_base.clone()
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.payload.clone()
    }

    fn success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) =
            call.standard_logging_object;
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }
}

pub(crate) struct WallClock;

impl SuccessClock for WallClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

pub(crate) fn payload_fixture(
    id: &str,
    model: &str,
    api_base: &str,
) -> Option<StandardLoggingPayload> {
    let payload = json!({
        "id": id,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": "afile_content",
        "response_cost": 0.0_f64,
        "status": "success",
        "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
        "total_tokens": 0_i32,
        "prompt_tokens": 0_i32,
        "completion_tokens": 0_i32,
        "startTime": 1_767_225_600.0_f64,
        "endTime": 1_767_225_601.0_f64,
        "completionStartTime": 1_767_225_600.0_f64,
        "response_time": 1.0_f64,
        "model_map_information": {},
        "model": model,
        "api_base": api_base,
        "metadata": {},
        "saved_cache_cost": 0.0_f64,
        "request_tags": [],
        "model_parameters": {},
        "hidden_params": {},
    });
    serde_json::from_value(payload).ok()
}
