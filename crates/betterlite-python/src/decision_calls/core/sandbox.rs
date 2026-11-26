use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use betterlite_models::sandbox::CodeExecutionResult;
use betterlite_models::sandbox_dispatch::sandbox_tool_entries;
use betterlite_provider::sandbox_dispatch::request::SandboxContainer;

use super::super::convert::{Arguments, CallWire, encode};
use super::super::error::SessionFailure;
use super::super::sandbox::container::{container_answer, container_of};
use super::super::sandbox::run::run_request;
use super::super::sandbox::{create_request, delete_request, network_policy, tool_answer};
use super::{SessionCore, secret_lookup};

pub(super) fn register_tools(
    core: &SessionCore,
    wire: &CallWire,
) -> Result<String, SessionFailure> {
    let arguments = &wire.arguments;
    let entries = sandbox_tool_entries(arguments.tools.clone())?;
    let environment = arguments.environment.clone().unwrap_or_default();
    let lookup = secret_lookup();
    let mut tools = core.tools.lock().unwrap_or_else(PoisonError::into_inner);
    tools.register(&entries, &lookup, &environment)?;
    encode(&Json::Null).map_err(SessionFailure::from)
}

pub(super) fn resolve_tool(core: &SessionCore, wire: &CallWire) -> Result<String, SessionFailure> {
    let tools = core.tools.lock().unwrap_or_else(PoisonError::into_inner);
    let found = wire
        .arguments
        .name
        .as_deref()
        .map_or(None, |name| tools.tools.get(name));
    tool_answer(found).map_err(SessionFailure::from)
}

pub(super) async fn create_sandbox(
    core: &SessionCore,
    wire: &CallWire,
) -> Result<String, SessionFailure> {
    let policy = network_policy(wire.arguments.network_policy.as_ref())?;
    let mut request = create_request(&wire.arguments, policy.as_deref())?;
    let sandbox = core.sandbox();
    let container = sandbox
        .acreate_sandbox(&mut request)
        .await
        .map_err(SessionFailure::from)?;
    container_answer(&container).map_err(SessionFailure::from)
}

pub(super) async fn run_code(
    core: &SessionCore,
    wire: &CallWire,
) -> Result<String, SessionFailure> {
    let container = container_of(&wire.arguments)?;
    let result = run_on(core, &wire.arguments, &container).await?;
    encode(&result).map_err(SessionFailure::from)
}

pub(super) async fn delete_sandbox(
    core: &SessionCore,
    wire: &CallWire,
) -> Result<String, SessionFailure> {
    let container = container_of(&wire.arguments)?;
    let mut request = delete_request(&wire.arguments, &container);
    let answer = core.sandbox().adelete_sandbox(&mut request).await;
    encode(&answer?).map_err(SessionFailure::from)
}

pub(super) async fn code_interpreter(
    core: &SessionCore,
    wire: &CallWire,
) -> Result<String, SessionFailure> {
    let policy = network_policy(wire.arguments.network_policy.as_ref())?;
    let mut create = create_request(&wire.arguments, policy.as_deref())?;
    let sandbox = core.sandbox();
    let container = sandbox
        .acreate_sandbox(&mut create)
        .await
        .map_err(SessionFailure::from)?;
    let outcome = run_on(core, &wire.arguments, &container).await;
    let mut request = delete_request(&wire.arguments, &container);
    // LiteLLM logs the failed cleanup and still answers with the run's result
    let _cleanup = core.sandbox().adelete_sandbox(&mut request).await;
    encode(&outcome?).map_err(SessionFailure::from)
}

async fn run_on(
    core: &SessionCore,
    arguments: &Arguments,
    container: &SandboxContainer,
) -> Result<CodeExecutionResult, SessionFailure> {
    let mut request = run_request(arguments, container)?;
    let sandbox = core.sandbox();
    sandbox
        .arun_code(&mut request)
        .await
        .map_err(SessionFailure::from)
}
