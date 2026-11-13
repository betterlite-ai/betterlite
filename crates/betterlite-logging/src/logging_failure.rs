pub mod callbacks;
pub mod events;
pub mod handlers;
pub mod helper_fn;
pub mod partial_usage;
pub mod request_redaction;
pub mod special;

mod tests;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use thiserror::Error;

use crate::callback_logger::error::LoggerError;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::exporter_options::redaction::ResponseStamp;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::CorrelationContext;

pub const FAILED_API_CALL: &str = "failed_api_call";
pub const PASS_THROUGH_CALL_TYPE: &str = "pass_through_endpoint";
pub const NO_DEPLOYMENTS_AVAILABLE: &str = "No deployments available for selected model";
pub const FAILED_REQUEST_REASON: &str = "failed_request";

#[derive(Clone)]
pub struct FailureReport {
    pub exception: Arc<ExceptionFacts>,
    pub traceback: String,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub end_time: Option<DateTime<FixedOffset>>,
}

impl FailureReport {
    #[must_use]
    pub fn error_text(&self) -> String {
        let Some(message) = &self.exception.message else {
            return String::new();
        };
        message.clone()
    }
}

#[derive(Clone, Default)]
pub struct FailureRecord {
    pub exception: Option<Arc<ExceptionFacts>>,
    pub end_time: Option<DateTime<FixedOffset>>,
    pub payload: Option<Arc<StandardLoggingPayload>>,
}

pub struct FailureCall<'call> {
    pub state: &'call mut LoggingState,
    pub record: &'call mut FailureRecord,
    pub context: &'call mut CorrelationContext,
}

#[derive(Clone, Copy, Debug)]
pub struct FailureTiming {
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

pub trait FailureClock: Send + Sync {
    fn now(&self) -> DateTime<FixedOffset>;
}

pub struct FailurePayloadInput<'input> {
    pub state: &'input LoggingState,
    pub exception: &'input ExceptionFacts,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub error_str: &'input str,
}

pub trait FailurePayloadSource: Send + Sync {
    fn build(&self, input: FailurePayloadInput<'_>) -> Option<StandardLoggingPayload>;
}

#[derive(Debug, Error)]
pub enum BaselineInvalidationError {
    #[error("the baseline cache estimate could not be invalidated")]
    Invalidated,
}

#[async_trait]
pub trait BaselineInvalidator: Send + Sync {
    async fn invalidate_baseline(&self, reason: &str) -> Result<(), BaselineInvalidationError>;
}

#[derive(Debug, Error)]
pub enum PostCallFlushError {
    #[error("the post-call redis batches could not be flushed")]
    Flush,
}

#[async_trait]
pub trait PostCallFlush: Send + Sync {
    async fn flush_post_call(&self) -> Result<(), PostCallFlushError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAdmission {
    Accepted,
    Rejected,
}

pub trait SyncFailureExecutor: Send + Sync {
    fn enqueue_sync_failure(&self, job: handlers::SyncFailureJob) -> SyncAdmission;
}

pub struct RateLimitFailureEvent<'event> {
    pub exception: &'event ExceptionFacts,
    pub original_model_group: Option<&'event str>,
    pub details: &'event IndexMap<String, Json>,
}

#[async_trait]
pub trait RateLimitFailureListener: Send + Sync {
    fn callback_name(&self) -> &str;
    // ast-grep-ignore: no-long-names
    async fn log_model_group_rate_limit_error(
        &self,
        event: &RateLimitFailureEvent<'_>,
    ) -> Result<(), LoggerError>;
}

pub struct FailureDeps<'deps> {
    pub redactor: &'deps Redactor,
    pub payloads: &'deps dyn FailurePayloadSource,
    pub clock: &'deps dyn FailureClock,
    pub executor: &'deps dyn SyncFailureExecutor,
    pub baseline: Option<&'deps dyn BaselineInvalidator>,
    pub flush: Option<&'deps dyn PostCallFlush>,
    pub special_listeners: &'deps [Arc<dyn RateLimitFailureListener>],
    pub counters: Vec<&'deps (dyn CallbackFailureCounter + Send + Sync)>,
    pub excluded_fields: &'deps [String],
    pub stamp: ResponseStamp,
    pub privacy: Settings,
}

#[derive(Debug, Error)]
pub enum FailureError {
    #[error("the standard logging payload could not be encoded: {source}")]
    PayloadEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the standard logging payload could not be decoded: {source}")]
    PayloadDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the call's litellm_params must be an object to carry the exception headers")]
    LitellmParamsNotObject,
    #[error("the call's litellm_params.metadata must be an object to carry the exception headers")]
    MetadataNotObject,
}
