use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

// What a Python subclass sets on itself with class variables or by overriding a method
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardrailSwitches {
    pub use_native_during_call_hook: GuardrailFlag,
    pub use_native_lifecycle_hooks: GuardrailFlag,
    pub records_own_information: GuardrailFlag,
    pub uses_apply_guardrail_interface: GuardrailFlag,
    pub deny_text_on_error: GuardrailFlag,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailSettings {
    pub guardrail_name: Option<String>,
    pub supported_event_hooks: Option<Vec<GuardrailEventHooks>>,
    pub event_hook: Option<LitellmParamsMode>,
    pub default_on: GuardrailFlag,
    pub mask_request_content: GuardrailFlag,
    pub mask_response_content: GuardrailFlag,
    pub violation_message_template: Option<String>,
    pub end_session_after_n_fails: Option<i64>,
    pub on_violation: Option<String>,
    pub realtime_violation_message: Option<String>,
    pub on_sensitive_data: Option<String>,
    pub sensitive_data_route_to_model: Option<String>,
    pub sticky_session_routing: GuardrailFlag,
    pub run_in_parallel: GuardrailFlag,
    pub scan_raw_request: GuardrailFlag,
    pub only_scan_new_messages: GuardrailFlag,
    pub timeout: Option<f64>,
    pub switches: GuardrailSwitches,
}

impl Default for GuardrailSettings {
    fn default() -> Self {
        Self {
            guardrail_name: None,
            supported_event_hooks: None,
            event_hook: None,
            default_on: false,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: None,
            end_session_after_n_fails: None,
            on_violation: None,
            realtime_violation_message: None,
            on_sensitive_data: None,
            sensitive_data_route_to_model: None,
            sticky_session_routing: true,
            run_in_parallel: false,
            scan_raw_request: false,
            only_scan_new_messages: false,
            timeout: None,
            switches: GuardrailSwitches::default(),
        }
    }
}
