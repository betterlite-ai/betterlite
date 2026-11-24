#![cfg(test)]

use pyo3::prelude::*;

use super::register;

#[test]
fn register_adds_the_assistant_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeAssistantSession").unwrap());
        assert!(module.hasattr("NativeAssistantError").unwrap());
    });
}
