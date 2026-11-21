use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_bedrock::model_routes::{ParamMap, RouteError};
use betterlite_bedrock::reasoning_stream::buffered::{
    ReplyInput, RuntimeReply, transform_response,
};
use betterlite_bedrock::reasoning_stream::refusals::{
    non_string_reasoning_effort, params_refused_for, params_refused_while_reasoning,
    reasoning_efforts_refused_for, without_refused_effort,
};
use betterlite_bedrock::reasoning_stream::stream::StreamingHandler;
use betterlite_bedrock::reasoning_stream::{
    ReasoningTagState, decode_reasoning_delta, split_reasoning_tag,
};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::compatible_wire::{LogInput, NEW_ID, ReplyOptions, system_clock};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

const INIT: &str = r#"{"model":"openai.gpt-oss-20b-1:0","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    step: Step,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Split {
        fragments: Vec<String>,
        messages: Vec<String>,
    },
    Chunks {
        chunks: Vec<Value>,
    },
    Effort {
        model: String,
        params: ParamMap,
    },
    Refusals {
        models: Vec<String>,
        params: ParamMap,
    },
    Buffered {
        reply: Value,
        headers: IndexMap<String, String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match &input.step {
            Step::Split {
                fragments,
                messages,
            } => Ok(split(fragments, messages)),
            Step::Chunks { chunks } => chunks(chunks),
            Step::Effort { model, params } => effort(model, params),
            Step::Refusals { models, params } => Ok(refusals(models, params)),
            Step::Buffered { reply, headers } => buffered(reply, headers),
        }
    }
}

fn sorted(names: BTreeSet<String>) -> Vec<String> {
    names.into_iter().collect()
}

fn view(state: &ReasoningTagState, reasoning: &str, content: &str) -> Value {
    json!({
        "phase": state.phase.name(),
        "pending": state.pending.as_str(),
        "reasoning": reasoning,
        "content": content,
    })
}

fn split(fragments: &[String], messages: &[String]) -> Value {
    let mut state = ReasoningTagState::default();
    let mut feeds = Vec::new();
    for fragment in fragments {
        let (next, reasoning, content) =
            decode_reasoning_delta(&state, Some(fragment.as_str()), false);
        state = next;
        feeds.push(view(&state, &reasoning, &content));
    }
    let (drained, flushed_reasoning, flushed_content) = decode_reasoning_delta(&state, None, true);
    let (again, again_reasoning, again_content) = decode_reasoning_delta(&drained, None, true);
    let views: Vec<Value> = messages
        .iter()
        .map(|message| {
            let (reasoning, content) = split_reasoning_tag(message);
            json!({"reasoning": reasoning, "content": content})
        })
        .collect();
    json!({
        "feeds": feeds,
        "flush": view(&drained, &flushed_reasoning, &flushed_content),
        "flush_again": view(&again, &again_reasoning, &again_content),
        "messages": views,
    })
}

fn chunk_view(reply: &StreamReply) -> Value {
    let choices: Vec<Value> = reply
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": &choice.finish_reason,
                "role": &choice.delta.role,
                "content": &choice.delta.content,
                "reasoning_content": &choice.delta.reasoning_content,
                "tool_calls": &choice.delta.tool_calls,
            })
        })
        .collect();
    json!({
        "id": reply.id.as_str(),
        "created": reply.created,
        "model": &reply.model,
        "choices": choices,
        "usage": &reply.usage,
        "service_tier": &reply.service_tier,
    })
}

fn chunks(chunks: &[Value]) -> HelperResult {
    let mut handler = StreamingHandler::default();
    let mut views = Vec::new();
    for chunk in chunks {
        let mut parsed = chunk_parser(chunk, NEW_ID, system_clock).or_value_error()?;
        handler.chunk_parser(&mut parsed);
        views.push(chunk_view(&parsed));
    }
    Ok(Value::Array(views))
}

fn route_error(error: &RouteError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RouteError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn effort(model: &str, params: &ParamMap) -> HelperResult {
    let refused =
        params_refused_while_reasoning(model, params).map_err(|error| route_error(&error))?;
    Ok(json!({
        "non_string": sorted(non_string_reasoning_effort(params)),
        "refused_while_reasoning": sorted(refused),
    }))
}

fn refusals(models: &[String], params: &ParamMap) -> Value {
    let views: Vec<Value> = models
        .iter()
        .map(|model| {
            json!({
                "model": model,
                "refused_params": sorted(params_refused_for(model)),
                "refused_efforts": sorted(reasoning_efforts_refused_for(model)),
                "without_refused_effort": without_refused_effort(model, params),
            })
        })
        .collect();
    json!({"models": views})
}

struct QuietSink;

impl LogSink for QuietSink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).or_value_error()?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .or_value_error()
}

fn reply_env<'env>(
    redactor: &'env Redactor,
    sink: &'env QuietSink,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?,
    })
}

fn reply_view(split: &RuntimeReply, names: &IndexMap<String, String>) -> Value {
    let choices: Vec<Value> = split
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": &choice.finish_reason,
                "content": &choice.content,
                "reasoning_content": &choice.reasoning_content,
            })
        })
        .collect();
    let headers: IndexMap<&str, &str> = names
        .keys()
        .filter_map(|name| {
            split
                .headers
                .headers
                .get(name)
                .map(|value| (name.as_str(), value.as_str()))
        })
        .collect();
    json!({
        "id": split.response.id.as_str(),
        "created": split.response.created,
        "choices": choices,
        "headers": headers,
    })
}

fn buffered(reply: &Value, headers: &IndexMap<String, String>) -> HelperResult {
    let bytes = serde_json::to_vec(reply).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let mut state = logging_state(&redactor)?;
    let sink = QuietSink;
    let env = reply_env(&redactor, &sink)?;
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    };
    let input = LogInput {
        messages: Json::List(Vec::new()),
        data: RawBody::Object(IndexMap::new()),
    };
    let existing = AdditionalHeaders::new();
    let split = transform_response(
        &bytes,
        &mut state,
        &env,
        input,
        &ReplyInput {
            options,
            provider_headers: headers,
            existing_additional_headers: &existing,
        },
    )
    .or_value_error()?;
    Ok(reply_view(&split, headers))
}
