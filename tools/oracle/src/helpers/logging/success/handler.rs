use std::collections::BTreeMap;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;
use betterlite_logging::logging_success::dispatch::{
    // ast-grep-ignore: no-long-names
    handle_sync_success_callbacks_for_async_calls,
    // ast-grep-ignore: no-long-names
    should_run_sync_success_callbacks_for_async_calls,
};
use betterlite_logging::logging_success::handler::{async_success_handler, success_handler};
use betterlite_logging::logging_success::types::SuccessRequest;
use chrono::{DateTime, FixedOffset};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::compare::helper::HelperError;

use super::input::{ContextSpec, HandlerProbe, Mode, ResultKind, ResultSpec, SyncProbe};
use super::output;
use super::scenario::{self, Scenario, ScenarioSpec};
use super::state::{self, StateSpec};

pub(crate) fn handler(probes: &[HandlerProbe]) -> Result<Value, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|failure| super::value_error(&failure))?;
    let mut outputs = Vec::with_capacity(probes.len());
    for probe in probes {
        outputs.push(handler_probe(probe, &runtime)?);
    }
    Ok(json!({"operation": "handler", "probes": outputs}))
}

fn handler_probe(probe: &HandlerProbe, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let start = DateTime::parse_from_rfc3339(&probe.start_time)
        .map_err(|failure| super::value_error(&failure))?;
    let end = DateTime::parse_from_rfc3339(&probe.end_time)
        .map_err(|failure| super::value_error(&failure))?;
    let mut scenario = Scenario::new(ScenarioSpec {
        loggers: &probe.loggers,
        built_payload: probe.built_payload.as_ref(),
        complete: probe.complete_response.as_ref(),
        now: end,
        excluded_fields: probe.excluded_fields.clone(),
        stamp: probe.stamp.as_ref(),
    })?;
    scenario.assembler.declines_results = probe.assembler_returns_none.unwrap_or(false);
    scenario.privacy.turn_off_message_logging = probe.global_redaction.unwrap_or(false);
    let (mut state, mut context) = handler_state(probe, start)?;
    state.callback_dynamic_params = state::params(&probe.dynamic_params)?;
    let request = handler_request(probe, start, end)?;
    let logging = scenario.handler();
    match probe.mode {
        Mode::Sync => success_handler(&logging, &mut state, &mut context, &request),
        Mode::Async => runtime.block_on(async_success_handler(
            &logging,
            &mut state,
            &mut context,
            &request,
        )),
    }
    output::handler_value(&scenario, &state, &context)
}

fn handler_state(
    probe: &HandlerProbe,
    start: DateTime<FixedOffset>,
) -> Result<(LoggingState, CorrelationContext), Box<HelperError>> {
    let dynamic_success = Vec::new();
    let spec = StateSpec {
        stream: probe.stream,
        call_type: &probe.call_type,
        litellm_params: &probe.litellm_params,
        start_time: start,
        trace_id: probe.trace_id.as_deref(),
        session_id: probe.session_id.as_deref(),
        dynamic_success: &dynamic_success,
        context: &probe.context,
        request_correlation: probe.request_correlation,
    };
    state::build(&spec)
}

fn handler_request(
    probe: &HandlerProbe,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Result<SuccessRequest, Box<HelperError>> {
    Ok(SuccessRequest {
        result: scenario::result(&probe.result)?,
        start_time: Some(start),
        end_time: Some(end),
        cache_hit: probe.cache_hit,
        prefer_async_handlers: false,
        standard_logging_object: probe
            .payload
            .as_ref()
            .map(state::decode::<StandardLoggingPayload>)
            .transpose()?,
        guardrail_snapshot: None,
    })
}

pub(crate) fn sync_callbacks(probes: &[SyncProbe]) -> Result<Value, Box<HelperError>> {
    let mut outputs = Vec::with_capacity(probes.len());
    for probe in probes {
        outputs.push(sync_probe(probe)?);
    }
    Ok(json!({"operation": "sync_callbacks", "probes": outputs}))
}

fn sync_probe(probe: &SyncProbe) -> Result<Value, Box<HelperError>> {
    let request = sync_request()?;
    let mut scenario = Scenario::new(ScenarioSpec {
        loggers: &probe.loggers,
        built_payload: None,
        complete: None,
        now: scenario::default_time()?,
        excluded_fields: Vec::new(),
        stamp: None,
    })?;
    let known = probe.known.clone();
    scenario.known = Box::new(move |name: &str| known.iter().any(|candidate| candidate == name));
    let (mut state, mut context) = sync_state(probe)?;
    // ast-grep-ignore: no-long-names
    let should_run = should_run_sync_success_callbacks_for_async_calls(
        &state,
        &scenario.registry,
        scenario.known.as_ref(),
    );
    // ast-grep-ignore: no-long-names
    let ran = handle_sync_success_callbacks_for_async_calls(
        &scenario.handler(),
        &mut state,
        &mut context,
        &request,
    );
    Ok(json!({
        "should_run": should_run,
        "ran": ran,
        "events": output::events(&scenario.loggers)?,
    }))
}

fn sync_state(probe: &SyncProbe) -> Result<(LoggingState, CorrelationContext), Box<HelperError>> {
    let mut params = BTreeMap::new();
    params.insert("acompletion".to_owned(), Value::Bool(true));
    let context_spec = ContextSpec::default();
    state::build(&StateSpec {
        stream: false,
        call_type: "completion",
        litellm_params: &params,
        start_time: scenario::default_time()?,
        trace_id: None,
        session_id: None,
        dynamic_success: &probe.dynamic_success,
        context: &context_spec,
        request_correlation: false,
    })
}

fn sync_request() -> Result<SuccessRequest, Box<HelperError>> {
    Ok(SuccessRequest {
        result: scenario::result(&ResultSpec {
            kind: ResultKind::Chat,
            value: None,
        })?,
        start_time: None,
        end_time: None,
        cache_hit: None,
        prefer_async_handlers: false,
        standard_logging_object: None,
        guardrail_snapshot: None,
    })
}
