use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::tool_permission::{SUPPORTED_EVENT_HOOKS, ToolPermissionGuardrail};

// The base state LiteLLM's should_run_guardrail reads; the initializer stores a missing guardrail name as an empty one
#[must_use]
pub fn guardrail_base(guardrail: &ToolPermissionGuardrail) -> GuardrailBase {
    let args = &guardrail.args;
    GuardrailBase {
        settings: GuardrailSettings {
            guardrail_name: Some(args.name.clone()).filter(|name| !name.is_empty()),
            supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
            event_hook: Some(args.event_hook.clone()),
            default_on: args.default_on,
            mask_request_content: false,
            mask_response_content: false,
            violation_message_template: args.violation_message_template.clone(),
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
        },
        mode: None,
    }
}

mod tests;
