#[cfg(test)]
use std::future::{Future, ready};
#[cfg(test)]
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::time::Instant;

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
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
struct MockError(String);

#[cfg(test)]
#[derive(Clone)]
struct MockConfig {
    create: Result<Value, MockError>,
    run: Result<Value, MockError>,
    delete: Result<(), MockError>,
}

#[cfg(test)]
impl MockConfig {
    fn succeeding(container: Value) -> Self {
        Self {
            create: Ok(container.clone()),
            run: Ok(container),
            delete: Ok(()),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            create: Err(MockError(message.to_owned())),
            run: Err(MockError(message.to_owned())),
            delete: Err(MockError(message.to_owned())),
        }
    }
}

#[cfg(test)]
impl SandboxConfig for MockConfig {
    type Error = MockError;

    fn create(&self) -> impl Future<Output = Result<Value, MockError>> {
        ready(self.create.clone())
    }

    fn run(
        &self,
        _container: &Value,
        _code: &str,
    ) -> impl Future<Output = Result<Value, MockError>> {
        ready(self.run.clone())
    }

    fn delete(&self, _container: &Value) -> impl Future<Output = Result<(), MockError>> {
        ready(self.delete.clone())
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct MockRegistry {
    create_result: Option<Result<Value, MockError>>,
    run_result: Option<Result<Value, MockError>>,
    delete_result: Option<Result<(), MockError>>,
    delete_calls: Arc<Mutex<Vec<Value>>>,
}

#[cfg(test)]
impl MockRegistry {
    fn succeeding(container: Value) -> Self {
        Self {
            create_result: Some(Ok(container.clone())),
            run_result: Some(Ok(container)),
            delete_result: Some(Ok(())),
            delete_calls: Arc::default(),
        }
    }
}

#[cfg(test)]
impl SandboxRegistry for MockRegistry {
    type Error = MockError;

    fn create(
        &self,
        _params: &SandboxToolParams,
    ) -> impl Future<Output = Result<Value, MockError>> {
        ready(
            self.create_result
                .clone()
                .expect("create not configured on MockRegistry"),
        )
    }

    fn run(
        &self,
        _params: &SandboxToolParams,
        _container: &Value,
        _code: &str,
    ) -> impl Future<Output = Result<Value, MockError>> {
        ready(
            self.run_result
                .clone()
                .expect("run not configured on MockRegistry"),
        )
    }

    fn delete(
        &self,
        _params: &SandboxToolParams,
        container: &Value,
    ) -> impl Future<Output = Result<(), MockError>> {
        self.delete_calls.lock().unwrap().push(container.clone());
        ready(
            self.delete_result
                .clone()
                .expect("delete not configured on MockRegistry"),
        )
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
