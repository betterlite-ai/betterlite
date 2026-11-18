use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use crate::datadog::apm::Tracer;
use crate::datadog::payloads::assembly::assemble_payload;
use crate::datadog::payloads::status::DataDogStatus;
use crate::datadog::tagging::base_tags;
use crate::datadog_logging::batch::flush_queue;
use crate::datadog_logging::logger::DatadogLogger;
use crate::fields::payload::records::ErrorInformation;
use crate::logging_metadata::error_info::{ExceptionLogging, get_error_information};
use crate::private_json::error::PrivateJsonError;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

// LiteLLM's DatadogProxyFailureHookJsonMessage: the five fields its literal writes
#[derive(Serialize)]
struct ProxyFailureMessage {
    exception: String,
    error_class: String,
    status_code: Option<i64>,
    traceback: String,
    user_api_key_dict: IndexMap<String, Json>,
}

#[derive(Debug, Error)]
pub enum ProxyFailureError {
    #[error("the proxy failure message could not be encoded")]
    Message(#[from] PrivateJsonError),
}

// litellm.integrations.datadog.datadog.DataDogLogger.async_post_call_failure_hook
pub async fn async_post_call_failure_hook(
    logger: &DatadogLogger,
    // the logger's concrete NullTracer cannot report a span, so the caller passes the tracer its request runs under
    tracer: &(dyn Tracer + Send + Sync),
    logging: &ExceptionLogging<'_>,
    traceback_str: Option<&str>,
    caller_metadata: IndexMap<String, Json>,
) {
    let message = match proxy_failure_message(logging, traceback_str, caller_metadata) {
        Ok(message) => message,
        Err(error) => {
            tracing::error!(
                %error,
                "Datadog: async_post_call_failure_hook - the message could not be built"
            );
            return;
        }
    };
    let environment = &logger.settings.environment;
    let dd_payload = assemble_payload(
        environment,
        DataDogStatus::Error,
        &base_tags(environment),
        message,
        tracer,
    );
    let enqueued = logger.queue.enqueue(dd_payload);
    if enqueued.depth >= logger.settings.batch.batch_size {
        flush_queue(logger).await;
    }
}

fn proxy_failure_message(
    logging: &ExceptionLogging<'_>,
    traceback_str: Option<&str>,
    caller_metadata: IndexMap<String, Json>,
) -> Result<String, ProxyFailureError> {
    let information = get_error_information(logging, traceback_str);
    let status_code = numeric_status_code(&information);
    let message = ProxyFailureMessage {
        exception: information.error_message.flatten().unwrap_or_default(),
        error_class: information.error_class.flatten().unwrap_or_default(),
        status_code,
        traceback: information.traceback.flatten().unwrap_or_default(),
        user_api_key_dict: caller_metadata,
    };
    Ok(safe_dumps(&message, DEFAULT_MAX_RECURSE_DEPTH, None)?)
}

// error_information["error_code"] or ""; int() only when the stripped text is all digits
fn numeric_status_code(information: &ErrorInformation) -> Option<i64> {
    let Some(Some(code)) = information.error_code.as_ref() else {
        return None;
    };
    let digits = code.trim();
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}
