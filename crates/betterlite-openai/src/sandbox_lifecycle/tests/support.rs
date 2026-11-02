#[cfg(test)]
use std::future::{Future, ready};
#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use tokio::runtime::Builder;

#[cfg(test)]
use serde_json::{Map, Value};

#[cfg(test)]
use cache_operations::Clock;
#[cfg(test)]
use lifecycle::{SandboxConfig, SandboxRegistry};
#[cfg(test)]
use resolution::{SandboxToolParams, SandboxToolRegistry};

#[cfg(test)]
fn test_object<I>(pairs: I) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    Value::Object(Map::from_iter(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    ))
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
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
struct MockError(String);

#[cfg(test)]
fn record_call<T>(calls: &Arc<Mutex<Vec<T>>>, call: T) {
    calls.lock().unwrap().push(call);
}

#[cfg(test)]
fn record_and_succeed<T>(calls: &Arc<Mutex<Vec<T>>>, call: T) -> Result<(), MockError> {
    calls
        .lock()
        .map_err(|_poison| MockError("mock calls mutex was poisoned".to_owned()))?
        .push(call);
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MockConfigCalls {
    run: Arc<Mutex<Vec<(Value, String)>>>,
    delete: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
#[derive(Clone)]
struct MockConfig {
    create: Result<Value, MockError>,
    run: Result<Value, MockError>,
    delete: Result<(), MockError>,
    calls: MockConfigCalls,
}

#[cfg(test)]
impl MockConfig {
    fn succeeding(container: Value) -> Self {
        Self {
            create: Ok(container.clone()),
            run: Ok(container),
            delete: Ok(()),
            calls: MockConfigCalls::default(),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            create: Err(MockError(message.to_owned())),
            run: Err(MockError(message.to_owned())),
            delete: Err(MockError(message.to_owned())),
            calls: MockConfigCalls::default(),
        }
    }
}

#[cfg(test)]
impl SandboxConfig for MockConfig {
    type Error = MockError;

    fn create(&self) -> impl Future<Output = Result<Value, MockError>> {
        ready(self.create.clone())
    }

    fn run(&self, container: &Value, code: &str) -> impl Future<Output = Result<Value, MockError>> {
        record_call(&self.calls.run, (container.clone(), code.to_owned()));
        ready(self.run.clone())
    }

    fn delete(&self, container: &Value) -> impl Future<Output = Result<(), MockError>> {
        record_call(&self.calls.delete, container.clone());
        ready(self.delete.clone())
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MockRegistryCalls {
    create: Arc<Mutex<Vec<SandboxToolParams>>>,
    run: Arc<Mutex<Vec<(SandboxToolParams, Value, String)>>>,
    delete: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MockRegistry {
    create: Option<Value>,
    run: Option<Value>,
    calls: MockRegistryCalls,
}

#[cfg(test)]
impl MockRegistry {
    fn succeeding(container: Value) -> Self {
        Self {
            create: Some(container.clone()),
            run: Some(container),
            calls: MockRegistryCalls::default(),
        }
    }
}

#[cfg(test)]
fn mock_registry_value<V>(value: Option<&V>, operation: &str) -> V
where
    V: Clone,
{
    value
        .cloned()
        .unwrap_or_else(|| panic!("{operation} not configured on MockRegistry"))
}

#[cfg(test)]
impl SandboxRegistry for MockRegistry {
    type Error = MockError;

    fn create(&self, params: &SandboxToolParams) -> impl Future<Output = Result<Value, MockError>> {
        record_call(&self.calls.create, params.clone());
        ready(Ok(mock_registry_value(self.create.as_ref(), "create")))
    }

    fn run(
        &self,
        params: &SandboxToolParams,
        container: &Value,
        code: &str,
    ) -> impl Future<Output = Result<Value, MockError>> {
        record_call(
            &self.calls.run,
            (params.clone(), container.clone(), code.to_owned()),
        );
        ready(Ok(mock_registry_value(self.run.as_ref(), "run")))
    }

    fn delete(
        &self,
        _params: &SandboxToolParams,
        container: &Value,
    ) -> impl Future<Output = Result<(), MockError>> {
        ready(record_and_succeed(&self.calls.delete, container.clone()))
    }
}

#[cfg(test)]
#[derive(Default)]
struct MockToolRegistry {
    resolved: Option<Map<String, Value>>,
}

#[cfg(test)]
impl SandboxToolRegistry for MockToolRegistry {
    fn resolve(&self, _sandbox_tool_name: &str) -> Option<Map<String, Value>> {
        self.resolved.clone()
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
