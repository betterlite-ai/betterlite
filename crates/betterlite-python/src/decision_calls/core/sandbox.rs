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

impl SessionCore {
    pub(super) fn register_tools(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let arguments = &wire.arguments;
        let entries = sandbox_tool_entries(arguments.tools.clone())?;
        let environment = arguments.environment.clone().unwrap_or_default();
        let lookup = secret_lookup();
        self.tools
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .register(&entries, &lookup, &environment)?;
        encode(&Json::Null).map_err(SessionFailure::from)
    }

    pub(super) fn resolve_tool(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let tools = self.tools.lock().unwrap_or_else(PoisonError::into_inner);
        let found = tools.tools.get(&wire.arguments.name);
        tool_answer(found).map_err(SessionFailure::from)
    }

    pub(super) async fn create_sandbox(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let policy = network_policy(&wire.arguments.network_policy)?;
        let mut request = create_request(&wire.arguments, policy.as_deref())?;
        let sandbox = self.sandbox();
        let container = sandbox
            .acreate_sandbox(&mut request)
            .await
            .map_err(SessionFailure::from)?;
        container_answer(&container).map_err(SessionFailure::from)
    }

    pub(super) async fn run_code(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let container = container_of(&wire.arguments)?;
        let result = self.run_on(&wire.arguments, &container).await?;
        encode(&result).map_err(SessionFailure::from)
    }

    pub(super) async fn delete_sandbox(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let container = container_of(&wire.arguments)?;
        let mut request = delete_request(&wire.arguments, &container);
        let answer = self.sandbox().adelete_sandbox(&mut request).await;
        encode(&answer?).map_err(SessionFailure::from)
    }

    pub(super) async fn code_interpreter(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        let policy = network_policy(&wire.arguments.network_policy)?;
        let mut create = create_request(&wire.arguments, policy.as_deref())?;
        let sandbox = self.sandbox();
        let container = sandbox
            .acreate_sandbox(&mut create)
            .await
            .map_err(SessionFailure::from)?;
        let outcome = self.run_on(&wire.arguments, &container).await;
        let mut request = delete_request(&wire.arguments, &container);
        // LiteLLM logs the failed cleanup and still answers with the run's result
        let _cleanup = self.sandbox().adelete_sandbox(&mut request).await;
        encode(&outcome?).map_err(SessionFailure::from)
    }

    async fn run_on(
        &self,
        arguments: &Arguments,
        container: &SandboxContainer,
    ) -> Result<CodeExecutionResult, SessionFailure> {
        let mut request = run_request(arguments, container)?;
        let sandbox = self.sandbox();
        sandbox
            .arun_code(&mut request)
            .await
            .map_err(SessionFailure::from)
    }
}
