#![cfg(test)]

use pyo3::prelude::*;

use super::register;

#[test]
fn register_adds_every_a2a_name() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeA2aClient").unwrap());
        assert!(module.hasattr("NativeA2aStream").unwrap());
        assert!(module.hasattr("NativeA2aError").unwrap());
        assert!(module.hasattr("create_client").unwrap());
        assert!(module.hasattr("get_agent_card").unwrap());
        assert!(module.hasattr("send_message_direct").unwrap());
        assert!(module.hasattr("send_message_streaming_direct").unwrap());
        assert!(module.hasattr("is_localhost_or_internal_url").unwrap());
        assert!(module.hasattr("normalize_agent_card_interfaces").unwrap());
        assert!(module.hasattr("get_agent_card_url").unwrap());
        assert!(module.hasattr("set_agent_card_url").unwrap());
        assert!(module.hasattr("fix_agent_card_url").unwrap());
    });
}
