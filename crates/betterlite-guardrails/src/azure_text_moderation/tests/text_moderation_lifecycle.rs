use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::support::{guardrail, params};
use crate::azure_text_moderation::SUPPORTED_EVENT_HOOKS;
use crate::guardrail_base::contract::GuardrailContract;

#[test]
fn lists_pre_call_then_post_call() {
    let hook = guardrail("https://example.test", params(None, None));
    assert_eq!(
        hook.get_supported_event_hooks(),
        Some(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall
        ])
    );
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall]
    );
}
