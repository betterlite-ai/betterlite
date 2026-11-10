use std::collections::BTreeMap;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::assembled::{
    ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE, FINAL_STREAM_DISPATCH_MARKER,
    is_assembled_stream_success,
};
use betterlite_logging::logging_success::dispatch::dispatch_success_handlers;
use betterlite_logging::logging_success::recognized::is_recognized_call_type;
use betterlite_logging::logging_success::types::SuccessRequest;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::compare::helper::HelperError;

use super::input::{AssembledProbe, ContextSpec, DispatchProbe, Input, Operation, RecognizedProbe};
use super::scenario::{self, Scenario, ScenarioSpec};
use super::state::{self, StateSpec};
use super::{handler, output};

pub(crate) fn run(input: &Input) -> crate::helpers::HelperResult {
    let mut outputs = Vec::with_capacity(input.operations.len());
    for operation in &input.operations {
        outputs.push(operation_value(operation)?);
    }
    Ok(json!({"operations": outputs}))
}

fn operation_value(operation: &Operation) -> Result<Value, Box<HelperError>> {
    match operation {
        Operation::Assembled { probes } => assembled(probes),
        Operation::Recognized { probes } => recognized(probes),
        Operation::Dispatch { probes } => dispatch(probes),
        Operation::Handler { probes } => handler::handler(probes),
        Operation::SyncCallbacks { probes } => handler::sync_callbacks(probes),
    }
}

fn assembled(probes: &[AssembledProbe]) -> Result<Value, Box<HelperError>> {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        results.push(assembled_probe(probe)?);
    }
    Ok(json!({"operation": "assembled", "results": results}))
}

fn assembled_probe(probe: &AssembledProbe) -> Result<bool, Box<HelperError>> {
    let empty = BTreeMap::new();
    let context = ContextSpec::default();
    let (mut state, _) = state::build(&StateSpec {
        stream: probe.stream,
        call_type: "completion",
        litellm_params: &empty,
        start_time: scenario::default_time()?,
        trace_id: None,
        session_id: None,
        dynamic_success: &[],
        context: &context,
        request_correlation: false,
    })?;
    apply_key(
        &mut state,
        ASYNC_STREAM_RESPONSE_KEY,
        probe.async_key.as_deref(),
    );
    apply_key(
        &mut state,
        COMPLETE_STREAMING_RESPONSE,
        probe.complete_key.as_deref(),
    );
    Ok(is_assembled_stream_success(
        &state,
        &scenario::result(&probe.result)?,
    ))
}

fn recognized(probes: &[RecognizedProbe]) -> Result<Value, Box<HelperError>> {
    let decisions = probes
        .iter()
        .map(recognized_probe)
        .collect::<Result<Vec<bool>, Box<HelperError>>>()?;
    Ok(json!({"operation": "recognized", "results": decisions}))
}

fn recognized_probe(probe: &RecognizedProbe) -> Result<bool, Box<HelperError>> {
    Ok(is_recognized_call_type(
        &scenario::result(&probe.result)?,
        &probe.call_type,
    ))
}

fn dispatch(probes: &[DispatchProbe]) -> Result<Value, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|failure| super::value_error(&failure))?;
    let mut outputs = Vec::with_capacity(probes.len());
    for probe in probes {
        outputs.push(dispatch_probe(probe, &runtime)?);
    }
    Ok(json!({"operation": "dispatch", "probes": outputs}))
}

fn dispatch_probe(probe: &DispatchProbe, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let scenario = Scenario::new(ScenarioSpec {
        loggers: &probe.loggers,
        built_payload: probe.built_payload.as_ref(),
        complete: None,
        now: scenario::default_time()?,
        excluded_fields: Vec::new(),
        stamp: None,
    })?;
    let context_spec = ContextSpec::default();
    let (mut state, mut context) = state::build(&StateSpec {
        stream: probe.stream,
        call_type: &probe.call_type,
        litellm_params: &probe.litellm_params,
        start_time: scenario::default_time()?,
        trace_id: None,
        session_id: None,
        dynamic_success: &[],
        context: &context_spec,
        request_correlation: false,
    })?;
    if probe.marker == Some(true) {
        state
            .model_call_details
            .insert(FINAL_STREAM_DISPATCH_MARKER.to_owned(), Json::Bool(true));
    }
    let request = dispatch_request(probe)?;
    let logging = scenario.handler();
    for _ in 0..probe.dispatches {
        runtime.block_on(dispatch_success_handlers(
            &logging,
            &mut state,
            &mut context,
            &request,
        ));
    }
    let marker =
        state.model_call_details.get(FINAL_STREAM_DISPATCH_MARKER) == Some(&Json::Bool(true));
    output::dispatch_value(&scenario, marker)
}

fn dispatch_request(probe: &DispatchProbe) -> Result<SuccessRequest, Box<HelperError>> {
    Ok(SuccessRequest {
        result: scenario::result(&probe.result)?,
        start_time: None,
        end_time: None,
        cache_hit: None,
        prefer_async_handlers: probe.prefer_async_handlers,
        standard_logging_object: None,
        guardrail_snapshot: None,
    })
}

fn apply_key(state: &mut LoggingState, name: &str, state_key: Option<&str>) {
    let Some(value) = state_key else {
        return;
    };
    let stored = if value == "null" {
        Json::Null
    } else {
        Json::Text(value.to_owned())
    };
    state.model_call_details.insert(name.to_owned(), stored);
}
