use crate::custom_code::CustomCodeGuardrail;
use crate::custom_code::events::CustomCodeEventHook;

#[test]
fn get_supported_event_hooks_lists_all_six_hooks() {
    assert_eq!(
        CustomCodeGuardrail::get_supported_event_hooks(),
        [
            CustomCodeEventHook::PreCall,
            CustomCodeEventHook::DuringCall,
            CustomCodeEventHook::PostCall,
            CustomCodeEventHook::PreMcpCall,
            CustomCodeEventHook::DuringMcpCall,
            CustomCodeEventHook::LoggingOnly,
        ]
    );
}

#[test]
fn the_hooks_serialize_to_litellms_wire_names() {
    assert_eq!(
        serde_json::to_string(&CustomCodeGuardrail::get_supported_event_hooks()).unwrap(),
        r#"["pre_call","during_call","post_call","pre_mcp_call","during_mcp_call","logging_only"]"#
    );
}
