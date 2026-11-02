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
#[derive(Clone)]
struct MockConfig {
    create: Result<Value, MockError>,
    run: Result<Value, MockError>,
    delete: Result<(), MockError>,
    run_calls: Arc<Mutex<Vec<(Value, String)>>>,
    delete_calls: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
impl MockConfig {
    fn succeeding(container: Value) -> Self {
        Self {
            create: Ok(container.clone()),
            run: Ok(container),
            delete: Ok(()),
            run_calls: Arc::default(),
            delete_calls: Arc::default(),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            create: Err(MockError(message.to_owned())),
            run: Err(MockError(message.to_owned())),
            delete: Err(MockError(message.to_owned())),
            run_calls: Arc::default(),
            delete_calls: Arc::default(),
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
        self.run_calls
            .lock()
            .unwrap()
            .push((container.clone(), code.to_owned()));
        ready(self.run.clone())
    }

    fn delete(&self, container: &Value) -> impl Future<Output = Result<(), MockError>> {
        self.delete_calls.lock().unwrap().push(container.clone());
        ready(self.delete.clone())
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MockRegistry {
    create: Option<Value>,
    run: Option<Value>,
    create_calls: Arc<Mutex<Vec<SandboxToolParams>>>,
    run_calls: Arc<Mutex<Vec<(SandboxToolParams, Value, String)>>>,
    delete_calls: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
impl MockRegistry {
    fn succeeding(container: Value) -> Self {
        Self {
            create: Some(container.clone()),
            run: Some(container),
            create_calls: Arc::default(),
            run_calls: Arc::default(),
            delete_calls: Arc::default(),
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
        self.create_calls.lock().unwrap().push(params.clone());
        ready(Ok(mock_registry_value(self.create.as_ref(), "create")))
    }

    fn run(
        &self,
        params: &SandboxToolParams,
        container: &Value,
        code: &str,
    ) -> impl Future<Output = Result<Value, MockError>> {
        self.run_calls
            .lock()
            .unwrap()
            .push((params.clone(), container.clone(), code.to_owned()));
        ready(Ok(mock_registry_value(self.run.as_ref(), "run")))
    }

    fn delete(
        &self,
        _params: &SandboxToolParams,
        container: &Value,
    ) -> impl Future<Output = Result<(), MockError>> {
        self.delete_calls.lock().unwrap().push(container.clone());
        ready(Ok(()))
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
