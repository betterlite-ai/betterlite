#![cfg(test)]

use std::any::TypeId;

use betterlite_config::sections::ibm_guardrails::DetectorConfigModel;

use super::{IbmConfigModel, SUPPORTED_EVENT_HOOKS};
use crate::ibm_hooks::config::EventHook;

#[test]
fn get_config_model_returns_the_ibm_config_model() {
    assert_eq!(
        TypeId::of::<IbmConfigModel>(),
        TypeId::of::<DetectorConfigModel>()
    );
}

#[test]
fn get_supported_event_hooks_returns_pre_call_post_call_during_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            EventHook::PreCall,
            EventHook::PostCall,
            EventHook::DuringCall
        ]
    );
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).expect("serialize"),
        r#"["pre_call","post_call","during_call"]"#
    );
}
