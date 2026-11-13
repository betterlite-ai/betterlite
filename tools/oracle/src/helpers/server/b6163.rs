use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_server::guardrails::a2a_translation::{input, output, stream};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

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
    fn reply(&self) -> Option<&'scenario Vec<String>> {
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
    fn new(reply: Option<Vec<String>>) -> Self {
        Self {
            reply,
            calls: Mutex::new(Vec::new()),
        }
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
        self.calls.lock().unwrap().push(json!({
            "input_type": input_type,
            "inputs": serde_json::to_value(&inputs).unwrap_or(Value::Null),
            "request_data": serde_json::to_value(&*request_data).unwrap_or(Value::Null),
        }));
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

    fn run(input_value: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .or_value_error()?;
        match input_value {
            Request::Input { scenarios } => {
                results(&runtime, scenarios.iter().map(Call::Input).collect())
            }
            Request::Output { scenarios } => {
                results(&runtime, scenarios.iter().map(Call::Output).collect())
            }
            Request::Stream { scenarios } => {
                results(&runtime, scenarios.iter().map(Call::Stream).collect())
            }
        }
    }
}

fn results(runtime: &tokio::runtime::Runtime, calls: Vec<Call<'_>>) -> HelperResult {
    let results = calls
        .iter()
        .map(|call| call_result(runtime, call))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(results))
}

fn call_result(runtime: &tokio::runtime::Runtime, call: &Call<'_>) -> HelperResult {
    let guardrail = ScriptedGuardrail::new(call.reply().cloned());
    match call {
        Call::Input(scenario) => {
            let mut data: BTreeMap<String, GuardrailValue> =
                serde_json::from_value(Value::Object(scenario.data.clone())).or_value_error()?;
            runtime
                .block_on(input::process_input_messages(&mut data, &guardrail, None))
                .or_value_error()?;
            let calls = guardrail.calls.lock().unwrap().clone();
            Ok(finished(
                "data",
                serde_json::to_value(&data).or_value_error()?,
                calls,
            ))
        }
        Call::Output(scenario) => {
            let mut response: GuardrailValue =
                serde_json::from_value(scenario.response.clone()).or_value_error()?;
            let mut request_data = request_data(&scenario.request_data)?;
            runtime
                .block_on(output::process_output_response(
                    &mut response,
                    &guardrail,
                    None,
                    None,
                    request_data.as_mut(),
                ))
                .or_value_error()?;
            let calls = guardrail.calls.lock().unwrap().clone();
            Ok(finished(
                "response",
                serde_json::to_value(&response).or_value_error()?,
                calls,
            ))
        }
        Call::Stream(scenario) => {
            let mut responses: Vec<GuardrailValue> = scenario
                .responses
                .iter()
                .cloned()
                .map(serde_json::from_value)
                .collect::<Result<Vec<_>, _>>()
                .or_value_error()?;
            let mut request_data = request_data(&scenario.request_data)?;
            runtime
                .block_on(stream::process_streaming_response(
                    &mut responses,
                    &guardrail,
                    None,
                    None,
                    request_data.as_mut(),
                ))
                .or_value_error()?;
            let calls = guardrail.calls.lock().unwrap().clone();
            Ok(finished(
                "responses",
                serde_json::to_value(&responses).or_value_error()?,
                calls,
            ))
        }
    }
}

fn finished(key: &str, value: Value, calls: Vec<Value>) -> Value {
    let mut result = Map::new();
    result.insert(key.to_owned(), value);
    result.insert("calls".to_owned(), Value::Array(calls));
    Value::Object(result)
}

fn request_data(
    given: &Option<Map<String, Value>>,
) -> Result<Option<BTreeMap<String, GuardrailValue>>, Box<crate::helpers::HelperError>> {
    given
        .as_ref()
        .map(|fields| serde_json::from_value(Value::Object(fields.clone())))
        .transpose()
        .or_value_error()
}
