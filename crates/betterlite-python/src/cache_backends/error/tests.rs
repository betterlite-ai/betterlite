#![cfg(test)]

use pyo3::PyErr;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::*;

fn value_error() -> BackendHandleError {
    BackendHandleError::NotWhole { value: 1.5_f64 }
}

fn runtime_error() -> BackendHandleError {
    BackendHandleError::IncrementUnavailable
}

#[test]
fn backends_error_value_kinds_are_value_errors() {
    Python::initialize();
    Python::attach(|py| {
        let error = value_error();
        assert!(error.is_value_error());
        let raised = PyErr::from(error);
        assert!(raised.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn backends_error_runtime_kinds_are_runtime_errors() {
    Python::initialize();
    Python::attach(|py| {
        let error = runtime_error();
        assert!(!error.is_value_error());
        let raised = PyErr::from(error);
        assert!(raised.is_instance_of::<PyRuntimeError>(py));
    });
}

#[test]
fn backends_error_converts_a_backend_failure() {
    Python::initialize();
    let converted = backend_err::<i32, BackendHandleError>(Ok(5_i32)).unwrap();
    assert_eq!(converted, 5_i32);
    let failure = BackendHandleError::NotWhole { value: 1.5_f64 };
    let raised = backend_err::<i32, BackendHandleError>(Err(failure)).unwrap_err();
    Python::attach(|py| assert!(raised.is_instance_of::<PyValueError>(py)));
}
