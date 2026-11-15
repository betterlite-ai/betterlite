#[cfg(test)]
use std::{
    future::{Future, ready},
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(test)]
use serde_json::{Map, Value};
#[cfg(test)]
use tokio::runtime::Builder;

#[cfg(test)]
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
#[cfg(test)]
use betterlite_models::catalog::ModelCatalog;

#[cfg(test)]
use crate::code_execution_records::{CODE_EXECUTION_TOOL_NAME, CodeExecutionToolCall};
#[cfg(test)]
use crate::code_interpreter_plans::initialization::InterceptionLogger;
#[cfg(test)]
use crate::code_interpreter_plans::request::PlanRequest;
#[cfg(test)]
use crate::sandbox_lifecycle::cache_operations::Clock;
#[cfg(test)]
use crate::sandbox_lifecycle::lifecycle::{SandboxConfig, SandboxRegistry};
#[cfg(test)]
use crate::sandbox_lifecycle::resolution::{SandboxToolParams, SandboxToolRegistry};

#[cfg(test)]
fn test_object<I>(pairs: I) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    Value::Object(test_map(pairs))
}

#[cfg(test)]
fn test_map<I>(pairs: I) -> Map<String, Value>
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    Map::from_iter(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    )
}

#[cfg(test)]
fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[cfg(test)]
fn metadata_value(metadata: &JsonMap, key: &str) -> Value {
    serde_json::from_str(metadata.get(key).unwrap().get()).unwrap()
}

#[cfg(test)]
fn followup_values(messages: &[FollowupMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| serde_json::from_str(message.0.get()).unwrap())
        .collect()
}

#[cfg(test)]
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
struct MockError(String);

#[cfg(test)]
#[derive(Clone, Default)]
struct MockConfigCalls {
    create: Arc<Mutex<Vec<()>>>,
    run: Arc<Mutex<Vec<(Value, String)>>>,
    delete: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
#[derive(Clone)]
struct MockConfig {
    create: Result<Value, MockError>,
    run: Result<Value, MockError>,
    calls: MockConfigCalls,
}

#[cfg(test)]
impl MockConfig {
    fn succeeding(container: Value, stdout: &str) -> Self {
        Self {
            create: Ok(container),
            run: Ok(test_object([("stdout", Value::String(stdout.to_owned()))])),
            calls: MockConfigCalls::default(),
        }
    }

    fn failing_run(container: Value) -> Self {
        Self {
            create: Ok(container),
            run: Err(MockError("sandbox run failed".to_owned())),
            calls: MockConfigCalls::default(),
        }
    }

    fn create_count(&self) -> usize {
        self.calls.create.lock().unwrap().len()
    }

    fn run_codes(&self) -> Vec<String> {
        self.calls
            .run
            .lock()
            .unwrap()
            .iter()
            .map(|(_, code)| code.clone())
            .collect()
    }

    fn deleted_containers(&self) -> Vec<Value> {
        self.calls.delete.lock().unwrap().clone()
    }
}

#[cfg(test)]
fn record_call<T>(calls: &Arc<Mutex<Vec<T>>>, call: T) {
    calls.lock().unwrap().push(call);
}

#[cfg(test)]
impl SandboxConfig for MockConfig {
    type Error = MockError;

    fn create(&self) -> impl Future<Output = Result<Value, MockError>> {
        record_call(&self.calls.create, ());
        ready(self.create.clone())
    }

    fn run(&self, container: &Value, code: &str) -> impl Future<Output = Result<Value, MockError>> {
        record_call(&self.calls.run, (container.clone(), code.to_owned()));
        ready(self.run.clone())
    }

    fn delete(&self, container: &Value) -> impl Future<Output = Result<(), MockError>> {
        record_call(&self.calls.delete, container.clone());
        ready(Ok(()))
    }
}

#[cfg(test)]
struct UnusedRegistry;

#[cfg(test)]
impl SandboxRegistry for UnusedRegistry {
    type Error = MockError;

    fn create(
        &self,
        _params: &SandboxToolParams,
    ) -> impl Future<Output = Result<Value, MockError>> {
        ready(Err(MockError("registry create is unused".to_owned())))
    }

    fn run(
        &self,
        _params: &SandboxToolParams,
        _container: &Value,
        _code: &str,
    ) -> impl Future<Output = Result<Value, MockError>> {
        ready(Err(MockError("registry run is unused".to_owned())))
    }

    fn delete(
        &self,
        _params: &SandboxToolParams,
        _container: &Value,
    ) -> impl Future<Output = Result<(), MockError>> {
        ready(Ok(()))
    }
}

#[cfg(test)]
struct UnusedToolRegistry;

#[cfg(test)]
impl SandboxToolRegistry for UnusedToolRegistry {
    fn resolve(&self, _sandbox_tool_name: &str) -> Option<Map<String, Value>> {
        None
    }
}

#[cfg(test)]
struct FixedClock(Instant);

#[cfg(test)]
impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

#[cfg(test)]
fn empty_logger<C>() -> InterceptionLogger<C> {
    InterceptionLogger::new(true, None, None, None, Arc::new(ModelCatalog::default()))
}

#[cfg(test)]
fn test_logger(config: MockConfig) -> InterceptionLogger<MockConfig> {
    InterceptionLogger::new(
        true,
        None,
        None,
        Some(config),
        Arc::new(ModelCatalog::default()),
    )
}

#[cfg(test)]
const fn plan_request<'request>(
    messages: &'request Value,
    optional_params: &'request Map<String, Value>,
    tool_calls: &'request [CodeExecutionToolCall],
    kwargs: &'request Map<String, Value>,
) -> PlanRequest<'request> {
    PlanRequest {
        model: "openai/gpt-5",
        messages,
        optional_params,
        tool_calls,
        kwargs,
    }
}

#[cfg(test)]
fn code_arguments(code: &str) -> String {
    test_object([("code", Value::String(code.to_owned()))]).to_string()
}

#[cfg(test)]
fn execution_tool_call(call_id: Option<&str>, code: &str) -> CodeExecutionToolCall {
    CodeExecutionToolCall {
        id: None,
        call_id: call_id.map(str::to_owned),
        kind: None,
        name: CODE_EXECUTION_TOOL_NAME.to_owned(),
        arguments: code_arguments(code),
    }
}

#[cfg(test)]
fn chat_tool_call(id: Option<&str>, call_id: Option<&str>, code: &str) -> CodeExecutionToolCall {
    CodeExecutionToolCall {
        id: id.map(str::to_owned),
        call_id: call_id.map(str::to_owned),
        kind: Some("function".to_owned()),
        name: CODE_EXECUTION_TOOL_NAME.to_owned(),
        arguments: code_arguments(code),
    }
}
