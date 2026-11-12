use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::converse_events::ConverseStreamState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    model: String,
    json_mode: bool,
    #[serde(default)]
    mappings: IndexMap<String, String>,
    ids: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    Converse { chunk: Json },
    Chunk { chunk: Json },
    CheckEmpty { content_blocks: Vec<Json> },
    State,
}

struct FixedIds {
    values: Mutex<VecDeque<Uuid>>,
}

impl FixedIds {
    fn parse(values: &[String]) -> Result<Self, Box<HelperError>> {
        let parsed = values
            .iter()
            .map(|value| Uuid::parse_str(value))
            .collect::<Result<VecDeque<Uuid>, _>>()
            .map_err(|error| failure(error.to_string()))?;
        Ok(Self {
            values: Mutex::new(parsed),
        })
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_else(Uuid::nil)
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::new();
        for scenario in &input.scenarios {
            scenarios.push(Value::Array(run_scenario(scenario)?));
        }
        Ok(json!({ "scenarios": scenarios }))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ConverseEvents".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn run_scenario(scenario: &Scenario) -> Result<Vec<Value>, Box<HelperError>> {
    let mut state = ConverseStreamState {
        content_blocks: Vec::new(),
        tool_calls_index: None,
        response_id: None,
        current_tool_name: None,
        thinking_ran: false,
        provider_reasoning_tokens: None,
        json_mode: scenario.json_mode,
        model: scenario.model.clone(),
        mappings: scenario.mappings.clone(),
        ids: Arc::new(FixedIds::parse(&scenario.ids)?),
    };
    Ok(scenario
        .steps
        .iter()
        .map(|step| run_step(&mut state, step))
        .collect())
}

fn run_step(state: &mut ConverseStreamState, step: &Step) -> Value {
    let projected = match step {
        Step::Converse { chunk } => match state.converse_chunk_parser(chunk) {
            Ok(parsed) => serde_json::to_value(parsed).map(|value| json!({ "converse": value })),
            Err(_) => return json!({ "failed": true }),
        },
        Step::Chunk { chunk } => match state.chunk_parser(chunk) {
            Ok(projection) => serde_json::to_value(projection),
            Err(_) => return json!({ "failed": true }),
        },
        Step::CheckEmpty { content_blocks } => {
            state.content_blocks.clone_from(content_blocks);
            match state.check_empty_tool_call_args() {
                Ok(empty) => Ok(json!({ "boolean": empty })),
                Err(_) => return json!({ "failed": true }),
            }
        }
        Step::State => return json!({ "ok": { "state": snapshot(state) } }),
    };
    projected.map_or_else(
        |_| json!({ "failed": true }),
        |value| json!({ "ok": value }),
    )
}

fn snapshot(state: &ConverseStreamState) -> Value {
    json!({
        "content_blocks": state.content_blocks,
        "tool_calls_index": state.tool_calls_index,
        "response_id": state.response_id,
        "current_tool_name": state.current_tool_name,
        "thinking_ran": state.thinking_ran,
        "provider_reasoning_tokens": state.provider_reasoning_tokens,
        "json_mode": state.json_mode,
    })
}
