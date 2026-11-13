use std::collections::BTreeMap;
use std::time::Duration;

use pyo3::exceptions::PyValueError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyResult, Python};

use crate::blocking_work::WorkTracker;
use crate::call_error::{CallError, PythonError};
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;
use crate::instance_loader::LoadedObject;
use crate::json_values::{JsonObject, JsonValue, from_object, to_object};

mod construct;
mod kind;
mod method;

#[derive(Debug)]
pub struct HookObject {
    pub loaded: LoadedObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedKind {
    Logger,
    Function,
    Class { module: String, qualname: String },
    Other { type_name: String, repr: String },
}

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("the loaded object has no method {name:?}")]
    NoSuchMethod { name: String },
    #[error("{path:?} did not resolve to a class")]
    NotClass { path: String },
    #[error("the keyword arguments for {path:?} must be a JSON object")]
    ArgumentsNotObject { path: String },
    #[error(transparent)]
    Python(#[from] PythonError),
    #[error(transparent)]
    Call(#[from] CallError),
}

pub struct MethodCall<'name> {
    pub name: &'name str,
    pub args: Vec<JsonValue>,
    pub kwargs: JsonObject,
}

pub struct RawMethodCall<'name> {
    pub name: &'name str,
    pub args: Vec<Py<PyAny>>,
    pub kwargs: BTreeMap<String, Py<PyAny>>,
}

impl HookObject {
    #[must_use]
    pub fn kind(&self, host: &PythonHost, logger_class: &str) -> LoadedKind {
        host.attach(|py| {
            let object = self.loaded.object.bind(py);
            kind::classify(py, object, logger_class)
        })
    }

    #[must_use]
    pub fn has_method(&self, host: &PythonHost, name: &str) -> bool {
        host.attach(|py| {
            let object = self.loaded.object.bind(py);
            method::callable_attribute(object, name).is_some()
        })
    }

    #[must_use]
    pub fn method(&self, host: &PythonHost, name: &str) -> Option<CallableHandle> {
        let attribute = host.attach(|py| {
            let object = self.loaded.object.bind(py);
            method::callable_attribute(object, name)
        })?;
        CallableHandle::new(host, attribute).ok()
    }

    pub async fn call_method(
        &self,
        host: &PythonHost,
        tracker: &WorkTracker,
        deadline: Duration,
        call: MethodCall<'_>,
    ) -> Result<JsonValue, HookError> {
        let raw = host.attach(|py| {
            let args = call
                .args
                .iter()
                .map(|value| to_object(py, value).map(Bound::unbind))
                .collect::<PyResult<Vec<_>>>()
                .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))?;
            let kwargs = call
                .kwargs
                .iter()
                .map(|(name, value)| Ok((name.clone(), to_object(py, value)?.unbind())))
                .collect::<PyResult<BTreeMap<_, _>>>()
                .map_err(|error| HookError::Python(PythonError::from_pyerr(py, &error)))?;
            Ok::<RawMethodCall<'_>, HookError>(RawMethodCall {
                name: call.name,
                args,
                kwargs,
            })
        })?;
        let value = self.call_method_raw(host, tracker, deadline, raw).await?;
        host.attach(|py| from_object(value.bind(py)))
            .map_err(CallError::Value)
            .map_err(HookError::Call)
    }

    pub async fn call_method_raw(
        &self,
        host: &PythonHost,
        tracker: &WorkTracker,
        deadline: Duration,
        call: RawMethodCall<'_>,
    ) -> Result<Py<PyAny>, HookError> {
        self.require_method(host, call.name)?
            .call_raw(host, tracker, deadline, call.args, call.kwargs)
            .await
            .map_err(HookError::Call)
    }

    fn require_method(&self, host: &PythonHost, name: &str) -> Result<CallableHandle, HookError> {
        self.method(host, name)
            .ok_or_else(|| HookError::NoSuchMethod {
                name: name.to_owned(),
            })
    }

    pub fn construct(
        host: &PythonHost,
        class_path: &str,
        kwargs: &JsonValue,
    ) -> Result<Py<PyAny>, HookError> {
        let JsonValue::Object(entries) = kwargs else {
            return Err(HookError::ArgumentsNotObject {
                path: class_path.to_owned(),
            });
        };
        host.attach(|py| construct::build(py, class_path, entries))
    }
}

fn class_object<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyAny>> {
    let (module_name, class_name) = path
        .rsplit_once('.')
        .ok_or_else(|| PyValueError::new_err(format!("{path:?} has no module part")))?;
    let module = py
        .import("importlib")?
        .call_method1("import_module", (module_name,))?;
    module.getattr(class_name)
}
