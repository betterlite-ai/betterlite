use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::kind::HookKind;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

#[derive(Debug, Clone, PartialEq)]
pub struct HookSettings {
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

impl HookSettings {
    #[must_use]
    pub fn from_params(params: &LitellmParams, guardrail_name: &str) -> Self {
        Self {
            guardrail_name: guardrail_name.to_owned(),
            event_hook: params.mode.clone(),
            default_on: params.base.fields.default_on.unwrap_or_default(),
            timeout: params.base.fields.timeout,
        }
    }
}

pub struct HookSpec {
    pub config: HookSettings,
    pub kind: HookKind,
}

impl HookSpec {
    #[must_use]
    pub fn new(settings: &HookSettings, kind: HookKind) -> Self {
        Self {
            config: settings.clone(),
            kind,
        }
    }

    // what every guardrail class's constructor passes to CustomGuardrail
    #[must_use]
    pub fn settings(&self) -> GuardrailSettings {
        GuardrailSettings {
            guardrail_name: Some(self.config.guardrail_name.clone()),
            supported_event_hooks: None,
            event_hook: Some(self.config.event_hook.clone()),
            default_on: self.config.default_on,
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
            timeout: self.config.timeout,
            switches: GuardrailSwitches::default(),
        }
    }
}
