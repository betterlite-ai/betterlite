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

#[derive(Deserialize, Default)]
pub(crate) struct KwargsInput {
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default)]
    messages: Option<Json>,
    #[serde(default)]
    functions: Option<Json>,
    #[serde(default)]
    tools: Option<Json>,
    #[serde(default)]
    input: Option<Json>,
}

impl From<&KwargsInput> for GalileoKwargs {
    fn from(value: &KwargsInput) -> Self {
        Self {
            call_type: value.call_type.clone(),
            messages: value.messages.clone(),
            functions: value.functions.clone(),
            tools: value.tools.clone(),
            input: value.input.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub(crate) enum ResponseInput {
    Chat {
        #[serde(default)]
        message: Option<Json>,
    },
    Embedding,
    Speech,
    TextCompletion {
        #[serde(default)]
        text: Option<String>,
    },
    Image {
        #[serde(default)]
        data: Option<Json>,
    },
    Transcription {
        #[serde(default)]
        text: Option<String>,
    },
    Rerank {
        results: Json,
    },
    ResponsesApi {
        output: Json,
    },
    RealtimeList {
        #[serde(default)]
        items: Vec<Json>,
    },
    PassThroughDict {
        #[serde(default)]
        value: IndexMap<String, Json>,
    },
    GenericDict {
        #[serde(default)]
        value: IndexMap<String, Json>,
    },
    Unknown,
}

impl From<&ResponseInput> for ResponseObj {
    fn from(response: &ResponseInput) -> Self {
        match response {
            ResponseInput::Chat { message } => Self::Chat {
                message: message.clone(),
            },
            ResponseInput::Embedding => Self::Embedding,
            ResponseInput::Speech => Self::Speech,
            ResponseInput::TextCompletion { text } => Self::TextCompletion { text: text.clone() },
            ResponseInput::Image { data } => Self::Image { data: data.clone() },
            ResponseInput::Transcription { text } => Self::Transcription { text: text.clone() },
            ResponseInput::Rerank { results } => Self::Rerank {
                results: results.clone(),
            },
            ResponseInput::ResponsesApi { output } => Self::ResponsesApi {
                output: output.clone(),
            },
            ResponseInput::RealtimeList { items } => Self::RealtimeList {
                items: items.clone(),
            },
            ResponseInput::PassThroughDict { value } => Self::PassThroughDict {
                value: value.clone(),
            },
            ResponseInput::GenericDict { value } => Self::GenericDict {
                value: value.clone(),
            },
            ResponseInput::Unknown => Self::Unknown,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LevelInput {
    Default,
    Error,
}

impl From<LevelInput> for Level {
    fn from(value: LevelInput) -> Self {
        match value {
            LevelInput::Default => Self::Default,
            LevelInput::Error => Self::Error,
        }
    }
}

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
        kwargs: Box<KwargsInput>,
        #[serde(default)]
        response: Option<ResponseInput>,
        #[serde(default)]
        level: Option<LevelInput>,
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
                level.map_or(Level::Default, Level::from),
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
    kwargs: &KwargsInput,
    response: Option<&ResponseInput>,
    level: Level,
    status_message: Option<&str>,
) -> serde_json::Value {
    let galileo_kwargs = GalileoKwargs::from(kwargs);
    let response_obj = response.map(ResponseObj::from);
    let (input_text, output_text, messages_for_span) = content::galileo_input_output_content(
        &galileo_kwargs,
        response_obj.as_ref(),
        level,
        status_message,
    );
    json!({
        "input_text": input_text,
        "output_text": output_text,
        "messages_for_span": messages_for_span,
    })
}
