pub mod action;
pub mod config;
pub mod error;
pub mod guardrail;
pub mod payload;
pub mod records;
pub mod transport;

use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use config::{NomaV2Params, NomaV2Settings};
use error::NomaV2Error;
use guardrail::NomaV2Guardrail;

pub const SCAN_ENDPOINT: &str = "/litellm/guardrail";
// The provider name every standard logging record from this guardrail carries
pub const PROVIDER: &str = "noma_v2";
pub const AUTHORIZATION_PREFIX: &str = "Bearer ";
pub const SAAS_HOSTNAME: &str = "api.noma.security";
pub const INTERVENED_INPUT_FIELDS: [&str; 4] = ["texts", "images", "tools", "tool_calls"];
pub const KEYS_DUPLICATING_SCAN_INPUTS: [&str; 2] = ["messages", "input"];
pub const LOGGING_DUPLICATE_INPUT_KEYS: [&str; 5] = [
    "messages",
    "input",
    "additional_args",
    "standard_logging_object",
    "original_response",
];
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 5] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
];

const NOMA_V2_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<NomaV2Guardrail>);
}

pub fn initialize_guardrail_v2<E, R>(
    params: &NomaV2Params,
    env: &E,
    client: reqwest::Client,
    registry: &mut R,
) -> Result<Arc<NomaV2Guardrail>, NomaV2Error>
where
    E: EnvironmentLookup,
    R: CallbackRegistry,
{
    let settings = NomaV2Settings::resolve(params, env)?;
    let base = GuardrailBase::new(guardrail_settings(params), params.strict_modes)?;
    let guardrail = Arc::new(NomaV2Guardrail {
        base,
        settings,
        client,
        clock: SystemTime::now,
    });
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

#[must_use]
pub fn guardrail_settings(params: &NomaV2Params) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: params.guardrail_name.clone(),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: params.mode.clone(),
        default_on: params.default_on,
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
        timeout: params.timeout,
        switches: NOMA_V2_SWITCHES,
    }
}
