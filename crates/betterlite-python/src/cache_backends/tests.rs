#![cfg(test)]

use pyo3::prelude::*;

use super::register;

#[test]
fn register_adds_the_backend_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeMemory").unwrap());
        assert!(module.hasattr("NativeDisk").unwrap());
        assert!(module.hasattr("NativeRedis").unwrap());
        assert!(module.hasattr("NativeDual").unwrap());
        assert!(module.hasattr("NativeDeferred").unwrap());
        assert!(module.hasattr("NativeDeclaredBatchRead").unwrap());
        assert!(module.hasattr("NativeRedisBatch").unwrap());
    });
}
