use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::guardrails::responses_translation::output_rewrite::{
    OutputContext, apply_texts_to_output, process_output_response,
};
use betterlite_server::guardrails::responses_translation::request_rewrite::error::RequestRewriteError;
use betterlite_server::guardrails::responses_translation::request_rewrite::{
    RequestContext, get_structured_messages, process_input_messages, scannable_instructions,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct DataScenario {
    data: Object,
    skip_system: bool,
    covers_full: bool,
    reply: Option<GuardrailApiInputs>,
}

#[derive(Deserialize)]
pub(crate) struct OutputScenario {
    response: Value,
    request_data: Option<Object>,
    reply: Option<GuardrailApiInputs>,
}

#[derive(Deserialize)]
pub(crate) struct ApplyScenario {
    response: Value,
    responses: Vec<String>,
    task_mappings: Vec<(usize, usize)>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    RequestView { scenarios: Vec<DataScenario> },
    Input { scenarios: Vec<DataScenario> },
    Output { scenarios: Vec<OutputScenario> },
    ApplyOutput { scenarios: Vec<ApplyScenario> },
}

struct ScriptedGuardrail {
    reply: Option<GuardrailApiInputs>,
    covers_full: bool,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedGuardrail {
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
        request_data: &mut Object,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let request_keys: Vec<&String> = request_data.keys().collect();
        let recorded = json!({
            "input_type": input_type,
            "inputs": serde_json::to_value(&inputs).unwrap_or(Value::Null),
            "request_keys": request_keys,
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        let answer = self.reply.clone().unwrap_or(inputs);
        async move { Ok(answer) }
    }

    fn messages_cover_full_request(&self) -> bool {
        self.covers_full
    }
}

// LiteLLM's tool call cache starts empty in the oracle's LiteLLM process, so the helper passes none
fn empty_cache() -> Result<DualLocalCache, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    Ok(DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    ))
}

fn input_answer(
    runtime: &Runtime,
    cache: &DualLocalCache,
    scenario: &DataScenario,
) -> HelperResult {
    let mut data = scenario.data.clone();
    let guardrail = ScriptedGuardrail {
        reply: scenario.reply.clone(),
        covers_full: scenario.covers_full,
        calls: Mutex::new(Vec::new()),
    };
    let context = RequestContext {
        skip_system_message: scenario.skip_system,
        guardrail_name: Some("guard"),
        limits: ApiLimits::default(),
        cache,
    };
    let outcome = runtime.block_on(process_input_messages(
        &mut data, &guardrail, &context, None,
    ));
    let data = serde_json::to_value(&data).or_value_error()?;
    let calls = guardrail.recorded();
    match outcome {
        Ok(()) => Ok(json!({"data": data, "calls": calls})),
        Err(RequestRewriteError::Unappliable(_)) => {
            Ok(json!({"data": data, "calls": calls, "error": "unappliable_request_rewrite"}))
        }
        Err(other) => Err(other).or_value_error(),
    }
}

fn output_answer(runtime: &Runtime, scenario: &OutputScenario) -> HelperResult {
    let mut response: GuardrailValue =
        serde_json::from_value(scenario.response.clone()).or_value_error()?;
    let mut stored = scenario.request_data.clone();
    let guardrail = ScriptedGuardrail {
        reply: scenario.reply.clone(),
        covers_full: false,
        calls: Mutex::new(Vec::new()),
    };
    let context = OutputContext {
        guardrail_name: Some("guard"),
        user_api_key_dict: None,
    };
    runtime
        .block_on(process_output_response(
            &mut response,
            &guardrail,
            &context,
            None,
            stored.as_mut(),
        ))
        .or_value_error()?;
    Ok(json!({
        "response": serde_json::to_value(&response).or_value_error()?,
        "calls": guardrail.recorded(),
    }))
}

fn request_view_answer(cache: &DualLocalCache, scenario: &DataScenario) -> HelperResult {
    let data = &scenario.data;
    let context = RequestContext {
        skip_system_message: scenario.skip_system,
        guardrail_name: None,
        limits: ApiLimits::default(),
        cache,
    };
    let messages = get_structured_messages(data, &context).or_value_error()?;
    let instructions = scannable_instructions(data, scenario.skip_system);
    Ok(json!({"messages": messages, "instructions": instructions}))
}

fn apply_answer(scenario: &ApplyScenario) -> HelperResult {
    let mut response = scenario.response.clone();
    apply_texts_to_output(&mut response, &scenario.responses, &scenario.task_mappings)
        .or_value_error()?;
    Ok(response)
}

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let cache = empty_cache()?;
        let answers: Result<Vec<Value>, Box<HelperError>> = match input {
            Function::RequestView { scenarios } => scenarios
                .iter()
                .map(|scenario| request_view_answer(&cache, scenario))
                .collect(),
            Function::Input { scenarios } => scenarios
                .iter()
                .map(|scenario| input_answer(&runtime, &cache, scenario))
                .collect(),
            Function::Output { scenarios } => scenarios
                .iter()
                .map(|scenario| output_answer(&runtime, scenario))
                .collect(),
            Function::ApplyOutput { scenarios } => scenarios.iter().map(apply_answer).collect(),
        };
        answers.map(Value::Array)
    }
}
