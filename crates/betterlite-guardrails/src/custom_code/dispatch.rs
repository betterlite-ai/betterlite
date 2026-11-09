use std::sync::Arc;
use std::sync::PoisonError;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::CompiledCode;
use super::CustomCodeGuardrail;
use super::error::CustomCodeGuardrailError;
use super::runtime::CallArguments;
use super::runtime::CallOutput;
use super::runtime::LoadFailure;
use super::runtime::LoadedModule;
use super::runtime::RuntimeFailure;
use crate::custom_code_execution::execution::error::ExecutionError;
use crate::custom_code_execution::execution::eventloop::await_with_timeout;
use crate::custom_code_execution::execution::offloop::call_off_loop_with_timeout;
use crate::custom_code_execution::execution::sync::call_with_timeout;
use crate::custom_code_execution::sandbox::CompiledProgram;
use crate::custom_code_execution::sandbox::globals::build_sandbox_globals;
use crate::generic_guardrail_contracts::GuardrailInputType;

// The module body runs under the same budget, so a loop at import time fails startup, not a request.
pub(super) fn load_module(
    guardrail: &CustomCodeGuardrail,
    program: &CompiledProgram,
) -> Result<Box<dyn LoadedModule>, ExecutionError<LoadFailure>> {
    let globals = build_sandbox_globals();
    let host = Arc::clone(&guardrail.host);
    let load_program = program.clone();
    let label = format!("{}:load", guardrail.guardrail_name);
    call_with_timeout(
        move || host.load(&load_program, &globals),
        guardrail.execution_budget,
        &label,
    )
}

pub(super) fn compiled_code(
    guardrail: &CustomCodeGuardrail,
) -> Result<Arc<CompiledCode>, CustomCodeGuardrailError> {
    let state = guardrail
        .compile_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    match (&state.code, &state.error) {
        (Some(code), _) => Ok(Arc::clone(code)),
        (None, Some(message)) => Err(CustomCodeGuardrailError::ExecutionMessage {
            message: format!("Custom code guardrail not compiled: {message}"),
        }),
        (None, None) => Err(CustomCodeGuardrailError::ExecutionMessage {
            message: "Custom code guardrail not compiled".to_owned(),
        }),
    }
}

pub(super) enum CallMode {
    Async,
    Sync,
}

// Async code awaits on the loop; a plain function runs off it and keeps the deadline's time left.
pub(super) async fn call(
    guardrail: &CustomCodeGuardrail,
    module: &Arc<dyn LoadedModule>,
    arguments: CallArguments,
    input_type: GuardrailInputType,
    mode: CallMode,
) -> Result<GuardrailValue, CustomCodeGuardrailError> {
    let (output, timeout) = match mode {
        CallMode::Async => (
            module.call(arguments).map_err(|source| {
                classify(guardrail, input_type, ExecutionError::Sandbox(source))
            })?,
            guardrail.execution_budget,
        ),
        CallMode::Sync => {
            let started = (guardrail.clock)();
            let shared_module = Arc::clone(module);
            let output = call_off_loop_with_timeout(
                move || shared_module.call(arguments),
                guardrail.execution_budget,
                &guardrail.guardrail_name,
            )
            .await
            .map_err(|source| classify(guardrail, input_type, source))?;
            let elapsed = (guardrail.clock)().saturating_sub(started);
            let remaining = guardrail.execution_budget.saturating_sub(elapsed);
            (output, remaining)
        }
    };
    match output {
        CallOutput::Value(value) => Ok(value),
        CallOutput::Pending(pending) => {
            await_with_timeout(pending, timeout, &guardrail.guardrail_name)
                .await
                .map_err(|source| classify(guardrail, input_type, source))
        }
    }
}

fn classify(
    guardrail: &CustomCodeGuardrail,
    input_type: GuardrailInputType,
    source: ExecutionError<RuntimeFailure>,
) -> CustomCodeGuardrailError {
    super::error::execution_error(
        &guardrail.guardrail_name,
        guardrail.execution_timeout,
        input_type,
        source,
    )
}
