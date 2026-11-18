use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

// LiteLLM's get_supported_event_hooks list
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

#[must_use]
pub const fn hook_name(event_type: GuardrailEventHooks) -> &'static str {
    match event_type {
        GuardrailEventHooks::PreCall => "pre_call",
        GuardrailEventHooks::DuringCall => "during_call",
        GuardrailEventHooks::PostCall => "post_call",
        GuardrailEventHooks::LoggingOnly => "logging_only",
        GuardrailEventHooks::PreMcpCall => "pre_mcp_call",
        GuardrailEventHooks::DuringMcpCall => "during_mcp_call",
        GuardrailEventHooks::PostMcpCall => "post_mcp_call",
        GuardrailEventHooks::RealtimeInputTranscription => "realtime_input_transcription",
    }
}
