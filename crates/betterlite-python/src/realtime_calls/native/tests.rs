#![cfg(test)]

use pyo3::Python;
use pyo3::types::{PyAnyMethods as _, PyModule};

use crate::realtime_calls::register;

#[test]
fn register_adds_the_realtime_session_and_error() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "realtime_test").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeRealtimeSession").unwrap());
        assert!(module.hasattr("NativeRealtimeError").unwrap());
    });
}
