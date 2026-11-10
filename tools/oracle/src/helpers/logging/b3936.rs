use betterlite_logging::datadog_llm_obs::messages::to_dd_messages;
use betterlite_logging::datadog_llm_privacy::cost_dimension::{
    cost_dimension_tags, declared_cost_tags,
};
use betterlite_logging::datadog_llm_privacy::guardrail_redact::redact_guardrails;
use betterlite_logging::datadog_llm_privacy::latency::{
    latency_metrics, time_to_first_token_seconds,
};
use betterlite_logging::datadog_llm_privacy::message_redact::redact_messages;
use betterlite_logging::datadog_llm_privacy::metadata::span_metadata;
use betterlite_logging::datadog_llm_privacy::record_object;
use betterlite_logging::datadog_llm_privacy::redaction_decision::LoggingFlags;
use betterlite_logging::datadog_llm_privacy::router_span::router_span_fields;
use betterlite_logging::datadog_llm_privacy::spend::spend_metrics;
use betterlite_logging::datadog_llm_privacy::stream::is_streamed_request;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RedactionDecision(DecisionInput),
    MessageRedaction(MessagesInput),
    GuardrailRedaction(GuardrailsInput),
    RouterSpanFields(RouterInput),
    CostDimensionTags(CostInput),
    TimeToFirstToken(TimingInput),
    LatencyMetrics(PayloadInput),
    StreamDetection(PayloadInput),
    SpendMetrics(PayloadInput),
    Metadata(MetadataInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionInput {
    flags: LoggingFlags,
    request_redacts: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesInput {
    messages: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardrailsInput {
    guardrail_information: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RouterInput {
    routing_decision: Option<Json>,
    redact_prompt_text: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostInput {
    payload: Box<StandardLoggingPayload>,
    #[serde(default)]
    span_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimingInput {
    start: Option<f64>,
    completion_start: Option<f64>,
    end: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadInput {
    payload: Box<StandardLoggingPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataInput {
    payload: Box<StandardLoggingPayload>,
    redact_prompt_text: bool,
    #[serde(default)]
    span_tags: Vec<String>,
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
            Input::RedactionDecision(decision) => encode(
                decision.flags.turn_off_message_logging
                    || !decision.flags.message_logging
                    || decision.request_redacts,
            ),
            Input::MessageRedaction(redaction) => encode(redact_messages(&to_dd_messages(
                redaction.messages.as_ref(),
            ))),
            Input::GuardrailRedaction(redaction) => {
                encode(redact_guardrails(redaction.guardrail_information.as_ref()))
            }
            Input::RouterSpanFields(fields) => encode(router_span_fields(
                fields.routing_decision.as_ref(),
                fields.redact_prompt_text,
            )),
            Input::CostDimensionTags(cost) => encode(cost_output(cost)),
            Input::TimeToFirstToken(timing) => encode(time_to_first_token_seconds(
                timing.start,
                timing.completion_start,
                timing.end,
            )),
            Input::LatencyMetrics(latency) => encode(
                record_object("latency_metrics", &latency_metrics(&latency.payload))
                    .or_value_error()?,
            ),
            Input::StreamDetection(stream) => encode(is_streamed_request(&stream.payload)),
            Input::SpendMetrics(spend) => encode(
                record_object("spend_metrics", &spend_metrics(&spend.payload)).or_value_error()?,
            ),
            Input::Metadata(metadata) => metadata_output(metadata),
        }
    }
}

fn cost_output(input: &CostInput) -> CostOutput {
    let router = router_span_fields(input.payload.metadata.routing_decision.as_ref(), false);
    let tags = cost_dimension_tags(&input.payload, &router);
    let mut all_tags = input.span_tags.clone();
    all_tags.extend_from_slice(&tags);
    CostOutput {
        declared: declared_cost_tags(&all_tags),
        tags,
    }
}

fn metadata_output(input: &MetadataInput) -> crate::helpers::HelperResult {
    let router = router_span_fields(
        input.payload.metadata.routing_decision.as_ref(),
        input.redact_prompt_text,
    );
    let mut all_tags = input.span_tags.clone();
    all_tags.extend(cost_dimension_tags(&input.payload, &router));
    let cost_tags = declared_cost_tags(&all_tags);
    let metadata = span_metadata(
        &input.payload,
        &router,
        &cost_tags,
        input.redact_prompt_text,
    )
    .or_value_error()?;
    encode(metadata)
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
