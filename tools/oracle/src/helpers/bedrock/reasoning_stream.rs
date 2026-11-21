use std::collections::BTreeSet;

use betterlite_bedrock::model_routes::{ParamMap, RouteError};
use betterlite_bedrock::reasoning_stream::refusals::{
    non_string_reasoning_effort, params_refused_for, params_refused_while_reasoning,
    reasoning_efforts_refused_for, without_refused_effort,
};
use betterlite_bedrock::reasoning_stream::stream::StreamingHandler;
use betterlite_bedrock::reasoning_stream::{
    ReasoningTagState, decode_reasoning_delta, split_reasoning_tag,
};
use betterlite_openai::chat_choices::stream::chunk_parser;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::compatible_wire::{NEW_ID, system_clock};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod buffered;
mod tests;

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
            Step::Buffered { reply, headers } => buffered::run(reply, headers),
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
    Value::Array(views)
}
