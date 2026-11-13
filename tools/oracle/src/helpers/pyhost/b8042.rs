use std::error::Error;
use std::ffi::CStr;
use std::path::Path;
use std::sync::Arc;

use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::callable_handle::CallableHandle;
use betterlite_pyhost::host::{HostConfig, PythonHost};
use betterlite_pyhost::instance_loader::{InstanceLoader, LoadError, LocalError};
use betterlite_pyhost::remote_module::{ModuleFetcher, ModuleRequest};
use betterlite_pyhost::return_check::{ReturnCheckError, check_literal_true_return};
use pyo3::Bound;
use pyo3::types::{PyAnyMethods as _, PyModule};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const LITERAL_TRUE_CODE: &CStr =
    c"from typing import Literal\n\ndef validator() -> Literal[True]:\n    return True\n";
const BOOL_CODE: &CStr = c"def validator() -> bool:\n    return True\n";
const PRINT_CODE: &CStr = c"validator = print\n";

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Load {
        value: String,
        #[serde(default)]
        config_file_path: Option<String>,
    },
    CheckReturn {
        fixture: Fixture,
    },
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Fixture {
    LiteralTrue,
    BoolAnnotation,
    NoAnnotation,
    NoneArgument,
}

struct NoFetch;

impl ModuleFetcher for NoFetch {
    type Error = std::io::Error;

    fn fetch(&self, _request: &ModuleRequest) -> Result<(), Self::Error> {
        Err(std::io::Error::other("the oracle helper downloads nothing"))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let host = PythonHost::start(&HostConfig::default())
            .map_err(|error| helper_error("PyHostError", error.to_string()))?;
        match input {
            Input::Load {
                value,
                config_file_path,
            } => load(&host, value, config_file_path.as_deref()),
            Input::CheckReturn { fixture } => check_return(&host, *fixture),
        }
    }
}

fn load(host: &PythonHost, value: &str, config_file_path: Option<&str>) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("PyHostError", error.to_string()))?;
    let fetcher: Arc<dyn ModuleFetcher<Error = std::io::Error>> = Arc::new(NoFetch);
    let loader = InstanceLoader::new(host.clone(), fetcher);
    let tracker = WorkTracker::default();
    let config = config_file_path.map(Path::new);
    match runtime.block_on(loader.load(&tracker, value, config)) {
        Ok(_loaded) => Ok(json!({"ok": true})),
        Err(error) => Ok(describe_error(&error)),
    }
}

fn describe_error<E>(error: &LoadError<E>) -> Value
where
    E: Error + Send + Sync + 'static,
{
    match error {
        LoadError::RemoteNeedsConfigPath => failure_json("ValueError", &error.to_string()),
        LoadError::Local(LocalError::Python(failure)) => {
            failure_json(&failure.type_name, &failure.message)
        }
        LoadError::Local(
            LocalError::Import { .. }
            | LocalError::UnsafeModulePath { .. }
            | LocalError::Work(_)
            | LocalError::PathNotUtf8 { .. },
        )
        | LoadError::Remote { .. } => failure_json("ImportError", &error.to_string()),
    }
}

fn failure_json(class: &str, text: &str) -> Value {
    json!({
        "ok": false,
        "type": class,
        "text": text,
    })
}

fn check_return(host: &PythonHost, fixture: Fixture) -> HelperResult {
    let handle = fixture_handle(host, fixture)?;
    match check_literal_true_return(host, handle.as_ref()) {
        Ok(Some(_)) => Ok(json!({"ok": true, "result": "function"})),
        Ok(None) => Ok(json!({"ok": true, "result": "none"})),
        Err(ReturnCheckError::Python(failure)) => {
            Ok(failure_json(&failure.type_name, &failure.message))
        }
    }
}

fn fixture_handle(
    host: &PythonHost,
    fixture: Fixture,
) -> Result<Option<CallableHandle>, Box<HelperError>> {
    match fixture {
        Fixture::LiteralTrue => Ok(Some(fixture_function(host, LITERAL_TRUE_CODE)?)),
        Fixture::BoolAnnotation => Ok(Some(fixture_function(host, BOOL_CODE)?)),
        Fixture::NoAnnotation => Ok(Some(fixture_function(host, PRINT_CODE)?)),
        Fixture::NoneArgument => Ok(None),
    }
}

fn fixture_function(host: &PythonHost, code: &CStr) -> Result<CallableHandle, Box<HelperError>> {
    let function = host
        .attach(|py| {
            let module = PyModule::from_code(py, code, c"b8042_fixture.py", c"b8042_fixture")?;
            module.getattr("validator").map(Bound::unbind)
        })
        .map_err(|error| helper_error("PythonError", error.to_string()))?;
    CallableHandle::new(host, function)
        .map_err(|error| helper_error("CallError", error.to_string()))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
