use betterlite_logging::galileo_payloads::content::{self, GalileoKwargs, Level, ResponseObj};
use betterlite_logging::galileo_payloads::timestamps::{
    RawTimestamp, format_created_at, normalize_created_at,
};
use betterlite_logging::galileo_payloads::{messages, metrics, trace};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    MessageNormalization {
        #[serde(default)]
        messages: Option<Json>,
        input_text: String,
    },
    TimestampFormats {
        naive: NaiveDateTime,
        offset_minutes: i32,
        aware: DateTime<FixedOffset>,
        normalize: Vec<String>,
    },
    TokenMetrics {
        record: IndexMap<String, Json>,
    },
    TraceEnvelope {
        record: IndexMap<String, Json>,
        trace_id: String,
        span_id: String,
        #[serde(default)]
        log_stream_id: Option<String>,
    },
    ContentSelection {
        #[serde(default)]
        kwargs: Box<GalileoKwargs>,
        #[serde(default)]
        response: Option<ResponseObj>,
        #[serde(default)]
        level: Option<Level>,
        #[serde(default)]
        status_message: Option<String>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MessageNormalization {
                messages,
                input_text,
            } => Ok(json!(messages::galileo_input_messages(
                messages.as_ref(),
                input_text
            ))),
            Input::TimestampFormats {
                naive,
                offset_minutes,
                aware,
                normalize,
            } => timestamp_formats(*naive, *offset_minutes, *aware, normalize),
            Input::TokenMetrics { record } => Ok(json!(metrics::token_metrics_json(
                metrics::token_metrics_from_record(record)
            ))),
            Input::TraceEnvelope {
                record,
                trace_id,
                span_id,
                log_stream_id,
            } => Ok(json!(trace::build_traces_payload(
                std::slice::from_ref(record),
                &[(trace_id.clone(), span_id.clone())],
                log_stream_id.as_deref(),
            ))),
            Input::ContentSelection {
                kwargs,
                response,
                level,
                status_message,
            } => Ok(content_selection(
                kwargs,
                response.as_ref(),
                level.unwrap_or(Level::Default),
                status_message.as_deref(),
            )),
        }
    }
}

fn timestamp_formats(
    naive: NaiveDateTime,
    offset_minutes: i32,
    aware: DateTime<FixedOffset>,
    normalize: &[String],
) -> HelperResult {
    let offset_seconds = offset_minutes.saturating_mul(60);
    let offset = FixedOffset::east_opt(offset_seconds).ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "offset_minutes out of range".to_owned(),
        })
    })?;
    let normalized: Vec<String> = normalize
        .iter()
        .map(|value| normalize_created_at(value))
        .collect();
    Ok(json!({
        "naive_formatted": format_created_at(RawTimestamp::Naive(naive), offset),
        "aware_formatted": format_created_at(RawTimestamp::Aware(aware), offset),
        "normalized": normalized,
    }))
}

fn content_selection(
    kwargs: &GalileoKwargs,
    response: Option<&ResponseObj>,
    level: Level,
    status_message: Option<&str>,
) -> serde_json::Value {
    let (input_text, output_text, messages_for_span) =
        content::galileo_input_output_content(kwargs, response, level, status_message);
    json!({
        "input_text": input_text,
        "output_text": output_text,
        "messages_for_span": messages_for_span,
    })
}
