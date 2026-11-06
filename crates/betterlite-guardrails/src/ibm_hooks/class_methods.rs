use betterlite_config::sections::ibm_guardrails::DetectorConfigModel;

use super::config::EventHook;

mod tests;

// Typed form of LiteLLM's IBMDetectorGuardrailConfigModel, which get_config_model returns.
pub type IbmConfigModel = DetectorConfigModel;

pub const SUPPORTED_EVENT_HOOKS: [EventHook; 3] = [
    EventHook::PreCall,
    EventHook::PostCall,
    EventHook::DuringCall,
];
