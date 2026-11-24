#![cfg(test)]

use pyo3::prelude::*;
use pyo3::types::{PyModule, PyType};

use super::register;
use crate::completion::error::NativeError;

#[test]
fn register_adds_the_session_class_and_the_error_alias() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "native").expect("the module builds");
        module
            .add("NativeError", module.py().get_type::<NativeError>())
            .expect("the error class registers");
        register(&module).expect("the search family registers");
        let session_class = module
            .getattr("NativeSearchSession")
            .expect("the session class is set");
        assert!(session_class.is_instance_of::<PyType>());
        let alias = module
            .getattr("NativeSearchError")
            .expect("the alias is set");
        let class = module
            .getattr("NativeError")
            .expect("the error class is set");
        assert!(alias.is(&class));
    });
}
