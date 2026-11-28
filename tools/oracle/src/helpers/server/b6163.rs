use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_server::guardrails::a2a_translation::{input, output, stream};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::{Builder, Runtime};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Request {
    Input { scenarios: Vec<InputScenario> },
    Output { scenarios: Vec<OutputScenario> },
    Stream { scenarios: Vec<StreamScenario> },
}

#[derive(Deserialize)]
pub(crate) struct InputScenario {
    data: Map<String, Value>,
    #[serde(default)]
    reply: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct OutputScenario {
    response: Value,
    #[serde(default)]
    request_data: Option<Map<String, Value>>,
    #[serde(default)]
    reply: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub(crate) struct StreamScenario {
    responses: Vec<Value>,
    #[serde(default)]
    request_data: Option<Map<String, Value>>,
    #[serde(default)]
    reply: Option<Vec<String>>,
}

enum Call<'scenario> {
    Input(&'scenario InputScenario),
    Output(&'scenario OutputScenario),
    Stream(&'scenario StreamScenario),
}

impl<'scenario> Call<'scenario> {
    const fn reply(&self) -> Option<&'scenario Vec<String>> {
        match self {
            Self::Input(scenario) => scenario.reply.as_ref(),
            Self::Output(scenario) => scenario.reply.as_ref(),
            Self::Stream(scenario) => scenario.reply.as_ref(),
        }
    }
}

struct ScriptedGuardrail {
    reply: Option<Vec<String>>,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedGuardrail {
    const fn new(reply: Option<Vec<String>>) -> Self {
        Self {
            reply,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(json!({
            "input_type": input_type,
            "inputs": serde_json::to_value(&inputs).unwrap_or(Value::Null),
            "request_data": serde_json::to_value(&*request_data).unwrap_or(Value::Null),
        }));
        drop(calls);
        let reply = self.reply.clone();
        async move {
            let mut outputs = inputs;
            if let Some(texts) = reply {
                outputs.texts = Some(texts);
            }
            Ok(outputs)
        }
    }
}

impl Helper for Op {
    type Input = Request;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        match input {
            Request::Input { scenarios } => {
                let entries = scenarios.iter().map(Call::Input).collect::<Vec<_>>();
                results(&runtime, &entries)
            }
            Request::Output { scenarios } => {
                let entries = scenarios.iter().map(Call::Output).collect::<Vec<_>>();
                results(&runtime, &entries)
            }
            Request::Stream { scenarios } => {
                let entries = scenarios.iter().map(Call::Stream).collect::<Vec<_>>();
                results(&runtime, &entries)
            }
        }
    }
}

fn results(runtime: &Runtime, entries: &[Call<'_>]) -> HelperResult {
    let results = entries
        .iter()
        .map(|entry| call_result(runtime, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(results))
}

fn call_result(runtime: &Runtime, entry: &Call<'_>) -> HelperResult {
    let guardrail = ScriptedGuardrail::new(entry.reply().cloned());
    match entry {
        Call::Input(scenario) => {
            let mut data: BTreeMap<String, GuardrailValue> =
                serde_json::from_value(Value::Object(scenario.data.clone())).or_value_error()?;
            let action = input::process_input_messages(&mut data, &guardrail, None);
            runtime.block_on(action).or_value_error()?;
            let calls = guardrail.recorded();
            Ok(json!({"data": serde_json::to_value(&data).or_value_error()?, "calls": calls}))
        }
        Call::Output(scenario) => {
            let mut response: GuardrailValue =
                serde_json::from_value(scenario.response.clone()).or_value_error()?;
            let mut stored = request_data(scenario.request_data.as_ref())?;
            let action = output::process_output_response(
                &mut response,
                &guardrail,
                None,
                None,
                stored.as_mut(),
            );
            runtime.block_on(action).or_value_error()?;
            let calls = guardrail.recorded();
            Ok(
                json!({"response": serde_json::to_value(&response).or_value_error()?, "calls": calls}),
            )
        }
        Call::Stream(scenario) => stream_result(runtime, scenario, &guardrail),
    }
}

fn stream_result(
    runtime: &Runtime,
    scenario: &StreamScenario,
    guardrail: &ScriptedGuardrail,
) -> HelperResult {
    let mut responses: Vec<StreamItem> = scenario
        .responses
        .iter()
        .cloned()
        .map(|value| serde_json::from_value(value).map(StreamItem::Json))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let mut stored = request_data(scenario.request_data.as_ref())?;
    let action =
        stream::process_streaming_response(&mut responses, guardrail, None, None, stored.as_mut());
    runtime.block_on(action).or_value_error()?;
    let calls = guardrail.recorded();
    let values: Vec<GuardrailValue> = responses.iter().map(stream_item_value).collect();
    Ok(json!({"responses": serde_json::to_value(&values).or_value_error()?, "calls": calls}))
}

fn request_data(
    given: Option<&Map<String, Value>>,
) -> Result<Option<BTreeMap<String, GuardrailValue>>, Box<HelperError>> {
    given
        .map(|fields| serde_json::from_value(Value::Object(fields.clone())))
        .transpose()
        .or_value_error()
}
