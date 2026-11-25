use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::pass_through_logging::anthropic_batch::{BatchLoggingResponse, ResponseIds};
use betterlite_server::pass_through_logging::vertex_batch_logging::{
    VertexBatchInput, VertexBatchLogged, VertexBatchLogging, VertexBatchState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const RESPONSE_ID: &str = "b3641-id";
const CALL_LOG: &str = r#"{"model":"","call_type":"pass_through_endpoint","start_time":"2026-01-01T00:00:00Z","litellm_call_id":"b3641-call","function_id":"b3641","litellm_trace_id":"b3641-trace"}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    status_code: u16,
    body: String,
    url_route: String,
    start_time: DateTime<Utc>,
    #[serde(default)]
    request_metadata: JsonObject,
}

#[derive(Serialize)]
struct Answer<'answer> {
    result: &'answer BatchLoggingResponse,
    kwargs: Kwargs<'answer>,
    call_log: CallLog<'answer>,
}

#[derive(Serialize)]
struct Kwargs<'answer> {
    response_cost: String,
    model: &'answer str,
    batch_id: Option<&'answer str>,
    unified_object_id: Option<&'answer str>,
    batch_job_state: VertexBatchState,
    request_metadata: &'answer JsonObject,
}

#[derive(Serialize)]
struct CallLog<'answer> {
    model: &'answer str,
    details: Details<'answer>,
}

#[derive(Serialize)]
struct Details<'answer> {
    model: Option<&'answer Json>,
    response_cost: Option<String>,
    batch_id: Option<&'answer Json>,
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
        let init: LoggingInit = serde_json::from_str(CALL_LOG).or_value_error()?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
        let setup = CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        };
        let mut call_log =
            LoggingState::new(init, setup, &redactor, String::new, || Duration::ZERO)
                .or_value_error()?;
        let logging = VertexBatchLogging::new(None, None, Arc::new(FixedIds));
        let batch_input = VertexBatchInput {
            status_code: input.status_code,
            body: input.body.as_bytes(),
            url_route: &input.url_route,
            start_time: input.start_time,
            request_metadata: &input.request_metadata,
        };
        let logged = logging.batch_prediction_jobs_handler(batch_input, &mut call_log);
        serde_json::to_value(answer(&logged, &call_log)).or_value_error()
    }
}

fn answer<'answer>(
    logged: &'answer VertexBatchLogged,
    call_log: &'answer LoggingState,
) -> Answer<'answer> {
    let details = &call_log.model_call_details;
    let detail_cost = match details.get("response_cost") {
        Some(Json::Number(number)) => Some(number.to_string()),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    Answer {
        result: &logged.result,
        kwargs: Kwargs {
            response_cost: logged.kwargs.response_cost.normalize().to_string(),
            model: &logged.kwargs.model,
            batch_id: logged.kwargs.batch_id.as_deref(),
            unified_object_id: logged.kwargs.unified_object_id.as_deref(),
            batch_job_state: logged.kwargs.batch_job_state,
            request_metadata: &logged.kwargs.request_metadata,
        },
        call_log: CallLog {
            model: &call_log.model,
            details: Details {
                model: details.get("model"),
                response_cost: detail_cost,
                batch_id: details.get("batch_id"),
            },
        },
    }
}
