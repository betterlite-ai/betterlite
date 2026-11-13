#![cfg(test)]

use std::ffi::CStr;

use pyo3::types::{PyAnyMethods as _, PyModule};

use super::{ReturnCheckError, check_literal_true_return};
use crate::callable_handle::CallableHandle;
use crate::host::{HostConfig, PythonHost};

const LITERAL_TRUE: &CStr =
    c"from typing import Literal\n\ndef validator() -> Literal[True]:\n    return True\n";
const LITERAL_ONE: &CStr =
    c"from typing import Literal\n\ndef validator() -> Literal[1]:\n    return 1\n";
const BOOL: &CStr = c"def validator() -> bool:\n    return True\n";
const NONE: &CStr = c"def validator() -> None:\n    return None\n";
const UNRESOLVED: &CStr = c"def validator() -> 'B8042Missing':\n    return True\n";

fn start_host() -> PythonHost {
    PythonHost::start(&HostConfig::default()).unwrap()
}

fn fixture(host: &PythonHost, code: &CStr) -> CallableHandle {
    let function = host.attach(|py| {
        PyModule::from_code(py, code, c"b8042_return_check.py", c"b8042_return_check")
            .unwrap()
            .getattr("validator")
            .unwrap()
            .unbind()
    });
    CallableHandle::new(host, function).unwrap()
}

fn failure(error: ReturnCheckError) -> (String, String) {
    let ReturnCheckError::Python(failure) = error;
    (failure.type_name, failure.message)
}

#[test]
fn accepts_a_function_annotated_to_return_literal_true() {
    let host = start_host();
    let handle = fixture(&host, LITERAL_TRUE);
    let accepted = check_literal_true_return(&host, Some(&handle))
        .unwrap()
        .unwrap();
    let same = host.attach(|py| accepted.bind(py).is(handle.object.bind(py)));
    assert!(same, "the check must return the function it accepted");
}

#[test]
fn refuses_a_bool_annotation() {
    let host = start_host();
    let handle = fixture(&host, BOOL);
    let (type_name, message) =
        failure(check_literal_true_return(&host, Some(&handle)).unwrap_err());
    assert_eq!(type_name, "TypeError");
    assert_eq!(
        message,
        "Custom validator must be annotated to return Literal[True], got <class 'bool'>"
    );
}

#[test]
fn refuses_a_function_without_a_return_annotation() {
    let host = start_host();
    let print = host.attach(|py| {
        py.import("builtins")
            .unwrap()
            .getattr("print")
            .unwrap()
            .unbind()
    });
    let handle = CallableHandle::new(&host, print).unwrap();
    let (type_name, message) =
        failure(check_literal_true_return(&host, Some(&handle)).unwrap_err());
    assert_eq!(type_name, "TypeError");
    assert_eq!(
        message,
        "Custom validator must be annotated to return Literal[True], got None"
    );
}

#[test]
fn refuses_a_none_return_annotation() {
    let host = start_host();
    let handle = fixture(&host, NONE);
    let (type_name, message) =
        failure(check_literal_true_return(&host, Some(&handle)).unwrap_err());
    assert_eq!(type_name, "TypeError");
    assert_eq!(
        message,
        "Custom validator must be annotated to return Literal[True], got <class 'NoneType'>"
    );
}

#[test]
fn refuses_a_literal_one_annotation() {
    let host = start_host();
    let handle = fixture(&host, LITERAL_ONE);
    let (type_name, message) =
        failure(check_literal_true_return(&host, Some(&handle)).unwrap_err());
    assert_eq!(type_name, "TypeError");
    assert_eq!(
        message,
        "Custom validator must be annotated to return Literal[True], got typing.Literal[1]"
    );
}

#[test]
fn passes_none_through() {
    let host = start_host();
    assert!(check_literal_true_return(&host, None).unwrap().is_none());
}

#[test]
fn reports_an_unresolvable_annotation_as_python_raises_it() {
    let host = start_host();
    let handle = fixture(&host, UNRESOLVED);
    let (type_name, message) =
        failure(check_literal_true_return(&host, Some(&handle)).unwrap_err());
    assert_eq!(type_name, "NameError");
    assert!(message.contains("B8042Missing"), "got: {message}");
}
