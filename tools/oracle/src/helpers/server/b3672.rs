use std::collections::BTreeMap;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::responses_translation::stream_scan::{
    streaming_scan_key, streaming_string_so_far,
};
// ast-grep-ignore: no-long-names
use betterlite_server::guardrails::responses_translation::stream_sync::{
    deliver_ended_stream_tool_call_rewrites, spread_text_rewrite_over_stream_events,
    sync_stream_events_with_rewrites,
};
use betterlite_server::guardrails::tool_call_identity::ToolCallShape;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct EventScenario {
    events: Vec<Value>,
}

#[derive(Deserialize)]
pub(crate) struct Rewrite {
    output_index: usize,
    content_index: usize,
    text: String,
}

#[derive(Deserialize)]
pub(crate) struct SyncScenario {
    events: Vec<Value>,
    rewrites: Vec<Rewrite>,
}

#[derive(Deserialize)]
pub(crate) struct SpreadScenario {
    events: Vec<Value>,
    rewritten_text: String,
    guardrail_name: String,
}

#[derive(Deserialize)]
pub(crate) struct ShapeInput {
    name: Option<String>,
    arguments: String,
}

#[derive(Deserialize)]
pub(crate) struct DeliverScenario {
    events: Vec<Value>,
    output: Vec<Value>,
    pre_guardrail_tool_calls: Vec<ShapeInput>,
    post_guardrail_tool_calls: Vec<ShapeInput>,
    guardrail_name: String,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    StringSoFar { scenarios: Vec<EventScenario> },
    ScanKey { scenarios: Vec<EventScenario> },
    Sync { scenarios: Vec<SyncScenario> },
    Spread { scenarios: Vec<SpreadScenario> },
    Deliver { scenarios: Vec<DeliverScenario> },
}

fn shapes(input: &[ShapeInput]) -> Vec<ToolCallShape> {
    input
        .iter()
        .map(|shape| ToolCallShape {
            name: shape.name.clone(),
            arguments: shape.arguments.clone(),
        })
        .collect()
}

fn sync_answer(scenario: &SyncScenario) -> Value {
    let mut events = scenario.events.clone();
    let mut rewrites = BTreeMap::new();
    for rewrite in &scenario.rewrites {
        rewrites.insert(
            (rewrite.output_index, rewrite.content_index),
            rewrite.text.clone(),
        );
    }
    // ast-grep-ignore: no-long-names
    sync_stream_events_with_rewrites(&mut events, &rewrites);
    json!({"events": events})
}

fn spread_answer(scenario: &SpreadScenario) -> Value {
    let mut events = scenario.events.clone();
    // ast-grep-ignore: no-long-names
    let outcome = spread_text_rewrite_over_stream_events(
        &mut events,
        &scenario.rewritten_text,
        &scenario.guardrail_name,
    );
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(failure) => Value::String(failure.to_string()),
    };
    json!({"events": events, "error": error})
}

fn deliver_answer(scenario: &DeliverScenario) -> Value {
    let mut events = scenario.events.clone();
    let mut output = scenario.output.clone();
    let pre = shapes(&scenario.pre_guardrail_tool_calls);
    let post = shapes(&scenario.post_guardrail_tool_calls);
    // ast-grep-ignore: no-long-names
    let outcome = deliver_ended_stream_tool_call_rewrites(
        &mut events,
        &mut output,
        &pre,
        &post,
        &scenario.guardrail_name,
    );
    let error = match outcome {
        Ok(()) => Value::Null,
        Err(failure) => Value::String(failure.to_string()),
    };
    json!({"events": events, "output": output, "error": error})
}

fn answers(input: &Function) -> Vec<Value> {
    match input {
        Function::StringSoFar { scenarios } => scenarios
            .iter()
            .map(|scenario| json!({"string": streaming_string_so_far(&scenario.events)}))
            .collect(),
        Function::ScanKey { scenarios } => scenarios
            .iter()
            .map(|scenario| json!({"scan_key": streaming_scan_key(&scenario.events)}))
            .collect(),
        Function::Sync { scenarios } => scenarios.iter().map(sync_answer).collect(),
        Function::Spread { scenarios } => scenarios.iter().map(spread_answer).collect(),
        Function::Deliver { scenarios } => scenarios.iter().map(deliver_answer).collect(),
    }
}

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::Array(answers(input)))
    }
}
