use betterlite_logging::datadog_llm_obs::messages::to_dd_messages;
use betterlite_logging::datadog_llm_obs::values::Object;
use betterlite_logging::datadog_llm_privacy::cost_dimension::{
    cost_dimension_tags, declared_cost_tags,
};
use betterlite_logging::datadog_llm_privacy::guardrail_redact::redact_guardrails;
use betterlite_logging::datadog_llm_privacy::latency::{
    latency_metrics, metrics_object as latency_object, time_to_first_token_seconds,
};
use betterlite_logging::datadog_llm_privacy::message_redact::redact_messages;
use betterlite_logging::datadog_llm_privacy::metadata::span_metadata;
use betterlite_logging::datadog_llm_privacy::redaction_decision::{
    LoggingFlags, payload_logging_is_off,
};
use betterlite_logging::datadog_llm_privacy::router_span::router_span_fields;
use betterlite_logging::datadog_llm_privacy::spend::{
    metrics_object as spend_object, spend_metrics,
};
use betterlite_logging::datadog_llm_privacy::stream::is_streamed_request;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RedactionDecision {
        turn_off_message_logging: bool,
        message_logging: bool,
        request_redacts: bool,
    },
    MessageRedaction {
        messages: Option<Json>,
    },
    GuardrailRedaction {
        guardrail_information: Option<Json>,
    },
    RouterSpanFields {
        routing_decision: Option<Json>,
        redact_prompt_text: bool,
    },
    CostDimensionTags {
        payload: Box<StandardLoggingPayload>,
        #[serde(default)]
        span_tags: Vec<String>,
    },
    TimeToFirstToken {
        start_time: Option<f64>,
        completion_start_time: Option<f64>,
        end_time: Option<f64>,
    },
    LatencyMetrics {
        payload: Box<StandardLoggingPayload>,
    },
    StreamDetection {
        payload: Box<StandardLoggingPayload>,
    },
    SpendMetrics {
        payload: Box<StandardLoggingPayload>,
    },
    Metadata {
        payload: Box<StandardLoggingPayload>,
        redact_prompt_text: bool,
        #[serde(default)]
        span_tags: Vec<String>,
    },
}

#[derive(Serialize)]
struct CostOutput {
    tags: Vec<String>,
    declared: Vec<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::RedactionDecision {
                turn_off_message_logging,
                message_logging,
                request_redacts,
            } => {
                let flags = LoggingFlags {
                    turn_off_message_logging: *turn_off_message_logging,
                    message_logging: *message_logging,
                };
                encode(payload_logging_is_off(flags, *request_redacts))
            }
            Input::MessageRedaction { messages } => {
                encode(redact_messages(&to_dd_messages(messages.as_ref())))
            }
            Input::GuardrailRedaction {
                guardrail_information,
            } => encode(redact_guardrails(guardrail_information.as_ref())),
            Input::RouterSpanFields {
                routing_decision,
                redact_prompt_text,
            } => encode(router_span_fields(
                routing_decision.as_ref(),
                *redact_prompt_text,
            )),
            Input::CostDimensionTags { payload, span_tags } => {
                encode(cost_output(payload, span_tags))
            }
            Input::TimeToFirstToken {
                start_time,
                completion_start_time,
                end_time,
            } => encode(time_to_first_token_seconds(
                *start_time,
                *completion_start_time,
                *end_time,
            )),
            Input::LatencyMetrics { payload } => encode(latency_object(&latency_metrics(payload))),
            Input::StreamDetection { payload } => encode(is_streamed_request(payload)),
            Input::SpendMetrics { payload } => encode(spend_object(&spend_metrics(payload))),
            Input::Metadata {
                payload,
                redact_prompt_text,
                span_tags,
            } => encode(metadata_output(payload, *redact_prompt_text, span_tags)),
        }
    }
}

fn cost_output(payload: &StandardLoggingPayload, span_tags: &[String]) -> CostOutput {
    let router = router_span_fields(payload.metadata.routing_decision.as_ref(), false);
    let tags = cost_dimension_tags(payload, &router);
    let mut all_tags = span_tags.to_vec();
    all_tags.extend_from_slice(&tags);
    CostOutput {
        declared: declared_cost_tags(&all_tags),
        tags,
    }
}

fn metadata_output(
    payload: &StandardLoggingPayload,
    redact_prompt_text: bool,
    span_tags: &[String],
) -> Object {
    let router = router_span_fields(
        payload.metadata.routing_decision.as_ref(),
        redact_prompt_text,
    );
    let mut all_tags = span_tags.to_vec();
    all_tags.extend(cost_dimension_tags(payload, &router));
    let cost_tags = declared_cost_tags(&all_tags);
    span_metadata(payload, &router, &cost_tags, redact_prompt_text)
}

fn encode<T>(value: T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(encoded) => Ok(encoded),
        Err(error) => Err(value_error(error)),
    }
}

fn value_error<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
