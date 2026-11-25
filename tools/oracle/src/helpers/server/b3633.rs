use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::pass_through_logging::anthropic_batch::{
    AnthropicBatchLogging, BatchCreationInput, BatchLoggingKwargs, ResponseIds,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

const RESPONSE_ID: &str = "b3633-id";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    status_code: u16,
    body: String,
    url_route: String,
    start_time: DateTime<Utc>,
    #[serde(default)]
    request_body: Option<Json>,
    #[serde(default)]
    request_metadata: JsonObject,
}

struct FixedIds;

impl ResponseIds for FixedIds {
    fn new_response_id(&self) -> String {
        RESPONSE_ID.to_owned()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let logging = AnthropicBatchLogging::new(None, None, Arc::new(FixedIds));
        let mut call_log = logging_state(input.start_time)?;
        let batch_input = BatchCreationInput {
            status_code: input.status_code,
            body: input.body.as_bytes(),
            url_route: &input.url_route,
            start_time: input.start_time,
            request_body: input.request_body.as_ref(),
            request_metadata: &input.request_metadata,
        };
        let logged = logging.batch_creation_handler(batch_input, &mut call_log);
        Ok(json!({
            "result": json!(&logged.result),
            "kwargs": kwargs_json(&logged.kwargs),
            "call_log": call_log_json(&call_log),
        }))
    }
}

fn kwargs_json(kwargs: &BatchLoggingKwargs) -> Value {
    json!({
        "response_cost": kwargs.response_cost.normalize().to_string(),
        "model": kwargs.model,
        "batch_id": kwargs.batch_id,
        "unified_object_id": kwargs.unified_object_id,
        "batch_job_state": kwargs.batch_job_state,
        "request_metadata": kwargs.request_metadata,
    })
}

fn call_log_json(call_log: &LoggingState) -> Value {
    json!({
        "model": call_log.model,
        "details": {
            "model": call_log.model_call_details.get("model"),
            "response_cost": detail_cost(&call_log.model_call_details),
            "batch_id": call_log.model_call_details.get("batch_id"),
        },
    })
}

fn detail_cost(details: &JsonObject) -> Value {
    match details.get("response_cost") {
        Some(Json::Number(number)) => json!(number.to_string()),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => Value::Null,
    }
}

fn logging_state(start_time: DateTime<Utc>) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(json!({
        "model": "",
        "call_type": "pass_through_endpoint",
        "start_time": start_time,
        "litellm_call_id": "b3633-call",
        "function_id": "b3633",
        "litellm_trace_id": "b3633-trace",
    }))
    .or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        || Duration::ZERO,
    )
    .or_value_error()
}
