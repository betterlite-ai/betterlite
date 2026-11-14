use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SettingsJson {
    pub(crate) guardrail_name: Option<String>,
    pub(crate) supported_event_hooks: Option<Vec<GuardrailEventHooks>>,
    pub(crate) event_hook: Option<LitellmParamsMode>,
    pub(crate) default_on: GuardrailFlag,
    pub(crate) mask_request_content: GuardrailFlag,
    pub(crate) mask_response_content: GuardrailFlag,
    pub(crate) violation_message_template: Option<String>,
    pub(crate) end_session_after_n_fails: Option<i64>,
    pub(crate) on_violation: Option<String>,
    pub(crate) realtime_violation_message: Option<String>,
    pub(crate) on_sensitive_data: Option<String>,
    pub(crate) sensitive_data_route_to_model: Option<String>,
    pub(crate) sticky_session_routing: GuardrailFlag,
    pub(crate) run_in_parallel: GuardrailFlag,
    pub(crate) scan_raw_request: GuardrailFlag,
    pub(crate) only_scan_new_messages: GuardrailFlag,
    pub(crate) timeout: Option<f64>,
    pub(crate) switches: SwitchesJson,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SwitchesJson {
    pub(crate) use_native_during_call_hook: GuardrailFlag,
    pub(crate) use_native_lifecycle_hooks: GuardrailFlag,
    pub(crate) records_own_information: GuardrailFlag,
    pub(crate) uses_apply_guardrail_interface: GuardrailFlag,
    pub(crate) deny_text_on_error: GuardrailFlag,
}

impl Default for SettingsJson {
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
            switches: SwitchesJson::default(),
        }
    }
}

impl From<&GuardrailSettings> for SettingsJson {
    fn from(settings: &GuardrailSettings) -> Self {
        let mut json = Self::default();
        json.guardrail_name.clone_from(&settings.guardrail_name);
        json.supported_event_hooks
            .clone_from(&settings.supported_event_hooks);
        json.event_hook.clone_from(&settings.event_hook);
        json.default_on = settings.default_on;
        json.mask_request_content = settings.mask_request_content;
        json.mask_response_content = settings.mask_response_content;
        json.violation_message_template
            .clone_from(&settings.violation_message_template);
        json.end_session_after_n_fails = settings.end_session_after_n_fails;
        json.on_violation.clone_from(&settings.on_violation);
        json.realtime_violation_message
            .clone_from(&settings.realtime_violation_message);
        json.on_sensitive_data
            .clone_from(&settings.on_sensitive_data);
        json.sensitive_data_route_to_model
            .clone_from(&settings.sensitive_data_route_to_model);
        json.sticky_session_routing = settings.sticky_session_routing;
        json.run_in_parallel = settings.run_in_parallel;
        json.scan_raw_request = settings.scan_raw_request;
        json.only_scan_new_messages = settings.only_scan_new_messages;
        json.timeout = settings.timeout;
        json.switches = SwitchesJson::from(&settings.switches);
        json
    }
}

impl From<&GuardrailSwitches> for SwitchesJson {
    fn from(switches: &GuardrailSwitches) -> Self {
        Self {
            use_native_during_call_hook: switches.use_native_during_call_hook,
            use_native_lifecycle_hooks: switches.use_native_lifecycle_hooks,
            records_own_information: switches.records_own_information,
            uses_apply_guardrail_interface: switches.uses_apply_guardrail_interface,
            deny_text_on_error: switches.deny_text_on_error,
        }
    }
}

impl SettingsJson {
    pub(crate) fn into_settings(self) -> GuardrailSettings {
        GuardrailSettings {
            guardrail_name: self.guardrail_name,
            supported_event_hooks: self.supported_event_hooks,
            event_hook: self.event_hook,
            default_on: self.default_on,
            mask_request_content: self.mask_request_content,
            mask_response_content: self.mask_response_content,
            violation_message_template: self.violation_message_template,
            end_session_after_n_fails: self.end_session_after_n_fails,
            on_violation: self.on_violation,
            realtime_violation_message: self.realtime_violation_message,
            on_sensitive_data: self.on_sensitive_data,
            sensitive_data_route_to_model: self.sensitive_data_route_to_model,
            sticky_session_routing: self.sticky_session_routing,
            run_in_parallel: self.run_in_parallel,
            scan_raw_request: self.scan_raw_request,
            only_scan_new_messages: self.only_scan_new_messages,
            timeout: self.timeout,
            switches: GuardrailSwitches {
                use_native_during_call_hook: self.switches.use_native_during_call_hook,
                use_native_lifecycle_hooks: self.switches.use_native_lifecycle_hooks,
                records_own_information: self.switches.records_own_information,
                uses_apply_guardrail_interface: self.switches.uses_apply_guardrail_interface,
                deny_text_on_error: self.switches.deny_text_on_error,
            },
        }
    }
}
