#![cfg(test)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::{NativeFailure, SessionError};
use crate::search_ocr::convert::ConvertError;

#[test]
fn a_malformed_session_text_raises_value_error() {
    Python::initialize();
    let source = serde_json::from_str::<i64>("not a number").expect_err("the text is no number");
    Python::attach(|py| {
        let error = PyErr::from(SessionError::Json(ConvertError::Json { source }));
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn a_convert_failure_reaches_python_as_value_error() {
    Python::initialize();
    let source = serde_json::from_str::<i64>("not a number").expect_err("the text is no number");
    Python::attach(|py| {
        let error = NativeFailure::Convert(ConvertError::Json { source }).into_pyerr();
        assert!(error.is_instance_of::<PyValueError>(py));
    });
}
